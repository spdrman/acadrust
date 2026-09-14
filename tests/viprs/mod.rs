//! Rendering an acadrust document in the VIPRS canonical record format, so it
//! can be diffed against a recorded ACadSharp dump.
//!
//! The format is not ours and is not negotiable here: it is whatever
//! `libviprs-dep` recorded, down to the six decimal places and the order of
//! the fields. Every formatting decision in this file exists to match a line
//! that already exists on disk over there.

#![allow(dead_code)]

use std::fmt::Write as _;
use std::path::Path;

use acadrust::entities::EntityType;
use acadrust::io::dwg::DwgReader;

/// A point, formatted the way the recording formats one.
fn p3(x: f64, y: f64, z: f64) -> String {
    // A transform readily produces negative zero and values a hair below it,
    // both of which format as "-0.000000" against the recording's
    // "0.000000". Anything under half the printed precision is that.
    let z6 = |v: f64| if v.abs() < 5e-7 { 0.0 } else { v };
    format!("({:.6},{:.6},{:.6})", z6(x), z6(y), z6(z))
}

fn v3(v: &acadrust::types::Vector3) -> String {
    p3(v.x, v.y, v.z)
}

/// DXF's arbitrary axis algorithm: the extrusion direction of an entity names
/// the plane its coordinates are measured in, and this is what turns one of
/// those coordinates into a world one.
///
/// The 1/64 test is from the DXF specification and is not a tolerance to be
/// tuned: it is the exact switch that decides which world axis is crossed with
/// the normal, and moving it changes the answer for every entity whose normal
/// is near the world Z axis.
pub struct Ocs {
    ax: [f64; 3],
    ay: [f64; 3],
    az: [f64; 3],
}

impl Ocs {
    pub fn new(n: &acadrust::types::Vector3) -> Self {
        let len = (n.x * n.x + n.y * n.y + n.z * n.z).sqrt();
        let az = if len == 0.0 {
            [0.0, 0.0, 1.0]
        } else {
            [n.x / len, n.y / len, n.z / len]
        };
        let pick = if az[0].abs() < 1.0 / 64.0 && az[1].abs() < 1.0 / 64.0 {
            [0.0, 1.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        let ax = norm(cross(pick, az));
        let ay = norm(cross(az, ax));
        Self { ax, ay, az }
    }

    /// True when this is the identity, which is the common case: an entity
    /// drawn in the world XY plane.
    pub fn is_identity(&self) -> bool {
        self.az == [0.0, 0.0, 1.0]
    }

    pub fn to_world(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        (
            self.ax[0] * x + self.ay[0] * y + self.az[0] * z,
            self.ax[1] * x + self.ay[1] * y + self.az[1] * z,
            self.ax[2] * x + self.ay[2] * y + self.az[2] * z,
        )
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l == 0.0 {
        v
    } else {
        [v[0] / l, v[1] / l, v[2] / l]
    }
}

/// One geometry record, as the recording spells it. `flags` is dropped from
/// the comparison deliberately: it marks block provenance, and this harness
/// does not expand blocks, so comparing it would report a difference about
/// something not being attempted.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub kind: String,
    pub handle: String,
    pub flags: u32,
    pub body: String,
}

impl Record {
    fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        // "NNNNN Kind handle=HH flags=F rest..."
        let mut it = line.splitn(3, ' ');
        let _seq = it.next()?;
        let kind = it.next()?.to_string();
        if kind == "Warning" {
            return None; // a claim about the adapter, not about the drawing
        }
        let rest = it.next()?;
        let handle = rest
            .split_whitespace()
            .find_map(|t| t.strip_prefix("handle="))?
            .to_string();
        let flags = rest
            .split_whitespace()
            .find_map(|t| t.strip_prefix("flags="))
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        let body = rest
            .split_whitespace()
            .filter(|t| !t.starts_with("handle=") && !t.starts_with("flags="))
            .collect::<Vec<_>>()
            .join(" ");
        Some(Record {
            kind,
            handle,
            flags,
            body,
        })
    }
}

pub enum Verdict {
    Match,
    Diff {
        n: usize,
        first: String,
    },
    Count {
        got: usize,
        want: usize,
        note: String,
    },
    OpenFailed(String),
    Uncompared(String),
}

impl Verdict {
    pub fn tag(&self) -> String {
        match self {
            Verdict::Match => "MATCH".into(),
            Verdict::Diff { n, .. } => format!("DIFF:{n}"),
            Verdict::Count { got, want, .. } => format!("COUNT:{got}/{want}"),
            Verdict::OpenFailed(_) => "OPEN_FAILED".into(),
            Verdict::Uncompared(_) => "UNCOMPARED".into(),
        }
    }

    pub fn detail(&self) -> Option<String> {
        match self {
            Verdict::Match => None,
            Verdict::Diff { first, .. } => Some(first.clone()),
            Verdict::Count { note, .. } => Some(note.clone()),
            Verdict::OpenFailed(e) => Some(e.clone()),
            Verdict::Uncompared(r) => Some(r.clone()),
        }
    }
}

/// Entity kinds this harness does not render, each for a stated reason. A
/// fixture containing one is reported UNCOMPARED rather than compared badly:
/// the recording expands blocks, lowers a hatch to its boundary polygons and a
/// dimension to the primitives of its block, and a harness that skipped that
/// work and then diffed the result would be reporting its own omissions as
/// acadrust's defects.
fn unrendered(e: &EntityType) -> Option<&'static str> {
    match e {
        EntityType::Hatch(_) => Some("HATCH: the recording lowers a hatch to boundary polygons"),
        EntityType::Dimension(_) => Some("DIMENSION: the recording expands the dimension block"),
        _ => None,
    }
}

/// A SOLID or a 3DFACE, as the recording lowers it.
///
/// SOLID stores its third and fourth corners swapped relative to traversal
/// order, so its points come out 1, 2, 4, 3 and the naive 1, 2, 3, 4 draws a
/// bow-tie. 3DFACE does NOT: its corners are already in order. Reading one
/// rule into both is a defect that a symmetric quad hides completely, which is
/// why g13_solid's first shape is asymmetric.
fn quad_record(
    p: &Placed,
    handle: String,
    c1: &acadrust::types::Vector3,
    c2: &acadrust::types::Vector3,
    c3: &acadrust::types::Vector3,
    c4: &acadrust::types::Vector3,
    normal: &acadrust::types::Vector3,
    swapped: bool,
    // SOLID stores its corners in the plane its normal defines, so they are
    // lifted. 3DFACE stores world coordinates and carries no extrusion at all;
    // lifting those through the face's own computed normal moves them.
    lift_ocs: bool,
) -> Option<Record> {
    let o = if lift_ocs {
        Ocs::new(normal)
    } else {
        Ocs::new(&acadrust::types::Vector3::new(0.0, 0.0, 1.0))
    };
    let lift = |v: &acadrust::types::Vector3| {
        let (x, y, z) = o.to_world(v.x, v.y, v.z);
        let (x, y, z) = p.at.point(x, y, z);
        p3(x, y, z)
    };
    // A degenerate fourth corner is the triangle case: the recording emits
    // three points rather than a coincident fourth.
    let degenerate =
        (c3.x - c4.x).abs() < 1e-12 && (c3.y - c4.y).abs() < 1e-12 && (c3.z - c4.z).abs() < 1e-12;
    let pts: Vec<String> = if degenerate {
        vec![lift(c1), lift(c2), lift(c3)]
    } else if swapped {
        vec![lift(c1), lift(c2), lift(c4), lift(c3)]
    } else {
        vec![lift(c1), lift(c2), lift(c3), lift(c4)]
    };
    let mut body = String::new();
    write!(
        body,
        "n={} closed=1 pts=[{}] bulges=[] normal=[{}]",
        pts.len(),
        pts.join(";"),
        v3(normal)
    )
    .ok()?;
    Some(Record {
        kind: "Polygon".into(),
        handle,
        flags: p.from_block as u32,
        body,
    })
}

/// Every record one placed entity lowers to. A MESH is one Polygon per face,
/// so this is a list rather than an option.
fn render(p: &Placed) -> Vec<Record> {
    let handle = format!("{:X}", p.entity.common().handle.value());
    match p.entity {
        EntityType::Solid(sd) => quad_record(
            p,
            handle,
            &sd.first_corner,
            &sd.second_corner,
            &sd.third_corner,
            &sd.fourth_corner,
            &sd.normal,
            true,
            true,
        )
        .into_iter()
        .collect(),
        EntityType::Face3D(f) => {
            // The face's own plane, the same way a MESH face carries one. A
            // 3DFACE lying in the world XY plane cannot tell this from world
            // Z, which is why g13_face3d holds one that does not.
            let e1 = [
                f.second_corner.x - f.first_corner.x,
                f.second_corner.y - f.first_corner.y,
                f.second_corner.z - f.first_corner.z,
            ];
            let e2 = [
                f.third_corner.x - f.first_corner.x,
                f.third_corner.y - f.first_corner.y,
                f.third_corner.z - f.first_corner.z,
            ];
            let n = norm(cross(e1, e2));
            let up = acadrust::types::Vector3::new(n[0], n[1], n[2]);
            quad_record(
                p,
                handle,
                &f.first_corner,
                &f.second_corner,
                &f.third_corner,
                &f.fourth_corner,
                &up,
                false,
                false,
            )
            .into_iter()
            .collect()
        }
        EntityType::Mesh(m) => {
            m.faces
                .iter()
                .filter_map(|face| {
                    let world: Vec<(f64, f64, f64)> = face
                        .vertices
                        .iter()
                        .filter_map(|i| m.vertices.get(*i))
                        .map(|v| p.at.point(v.x, v.y, v.z))
                        .collect();
                    if world.len() != face.vertices.len() || world.len() < 3 {
                        return None; // a face indexing a vertex that is not there
                    }
                    let pts: Vec<String> = world.iter().map(|w| p3(w.0, w.1, w.2)).collect();
                    // The face's own plane. The recording carries this rather
                    // than world Z, and a mesh whose faces are all coplanar
                    // cannot tell the two apart, which is why g13_mesh's are
                    // not.
                    let e1 = [
                        world[1].0 - world[0].0,
                        world[1].1 - world[0].1,
                        world[1].2 - world[0].2,
                    ];
                    let e2 = [
                        world[2].0 - world[0].0,
                        world[2].1 - world[0].1,
                        world[2].2 - world[0].2,
                    ];
                    let n = norm(cross(e1, e2));
                    let up = acadrust::types::Vector3::new(n[0], n[1], n[2]);
                    let mut body = String::new();
                    write!(
                        body,
                        "n={} closed=1 pts=[{}] bulges=[] normal=[{}]",
                        pts.len(),
                        pts.join(";"),
                        v3(&up)
                    )
                    .ok()?;
                    Some(Record {
                        kind: "Polygon".into(),
                        handle: handle.clone(),
                        flags: p.from_block as u32,
                        body,
                    })
                })
                .collect()
        }
        _ => render_one(p).into_iter().collect(),
    }
}

fn render_one(p: &Placed) -> Option<Record> {
    let e = p.entity;
    let at = &p.at;
    let handle = format!("{:X}", e.common().handle.value());
    let mut body = String::new();
    let kind = match e {
        EntityType::Line(l) => {
            let a = at.point(l.start.x, l.start.y, l.start.z);
            let b = at.point(l.end.x, l.end.y, l.end.z);
            write!(body, "pts=[{};{}]", p3(a.0, a.1, a.2), p3(b.0, b.1, b.2)).ok()?;
            "Line"
        }
        EntityType::Circle(c) => {
            // A circle under a non-uniform transform is an ellipse, and the
            // recording emits a NonUniform warning rather than a distorted
            // circle. Comparing here would report that disagreement as a
            // geometry difference, which it is not.
            if !at.uniform() {
                return None;
            }
            let o = Ocs::new(&c.normal);
            let (x, y, z) = o.to_world(c.center.x, c.center.y, c.center.z);
            let (x, y, z) = at.point(x, y, z);
            write!(
                body,
                "c=[{}] r={:.6} normal=[{}]",
                p3(x, y, z),
                c.radius * at.scale_hint(),
                v3(&c.normal)
            )
            .ok()?;
            "Circle"
        }
        EntityType::Arc(a) => {
            if !at.uniform() {
                return None;
            }
            let o = Ocs::new(&a.normal);
            let (x, y, z) = o.to_world(a.center.x, a.center.y, a.center.z);
            let (x, y, z) = at.point(x, y, z);
            write!(
                body,
                "c=[{}] r={:.6} a0={:.6} a1={:.6} normal=[{}]",
                p3(x, y, z),
                a.radius * at.scale_hint(),
                a.start_angle,
                a.end_angle,
                v3(&a.normal)
            )
            .ok()?;
            "Arc"
        }
        EntityType::Ellipse(el) => {
            write!(
                body,
                "c=[{}] major=[{}] ratio={:.6} p0={:.6} p1={:.6} normal=[{}]",
                v3(&el.center),
                v3(&el.major_axis),
                el.minor_axis_ratio,
                el.start_parameter,
                el.end_parameter,
                v3(&el.normal)
            )
            .ok()?;
            "Ellipse"
        }
        EntityType::LwPolyline(pl) => {
            let o = Ocs::new(&pl.normal);
            let pts: Vec<String> = pl
                .vertices
                .iter()
                .map(|v| {
                    let (x, y, z) = o.to_world(v.location.x, v.location.y, pl.elevation);
                    let (x, y, z) = at.point(x, y, z);
                    p3(x, y, z)
                })
                .collect();
            let bulges: Vec<String> = pl
                .vertices
                .iter()
                .map(|v| format!("{:.6}", v.bulge))
                .collect();
            let any = bulges.iter().any(|b| b != "0.000000");
            write!(
                body,
                "n={} closed={} pts=[{}] bulges=[{}] normal=[{}]",
                pts.len(),
                pl.is_closed as u8,
                pts.join(";"),
                if any { bulges.join(",") } else { String::new() },
                v3(&pl.normal)
            )
            .ok()?;
            "Polyline"
        }
        EntityType::Text(t) => {
            let o = Ocs::new(&t.normal);
            let (x, y, z) = o.to_world(
                t.insertion_point.x,
                t.insertion_point.y,
                t.insertion_point.z,
            );
            let (x, y, z) = at.point(x, y, z);
            write!(
                body,
                "p=[{}] h={:.6} rot={:.6} value=\"{}\"",
                p3(x, y, z),
                t.height * at.scale_hint(),
                t.rotation,
                t.value
            )
            .ok()?;
            "Text"
        }
        EntityType::Spline(s) => {
            let knots: Vec<String> = s.knots.iter().map(|k| format!("{:.6}", k)).collect();
            let ctrl: Vec<String> = s.control_points.iter().map(v3).collect();
            let weights: Vec<String> = s.weights.iter().map(|w| format!("{:.6}", w)).collect();
            // acadrust holds these as five bools; the recording prints the DXF
            // bitmask (70), so rebuild it rather than invent a spelling.
            let f = &s.flags;
            let bits = (f.closed as i32)
                | ((f.periodic as i32) << 1)
                | ((f.rational as i32) << 2)
                | ((f.planar as i32) << 3)
                | ((f.linear as i32) << 4);
            write!(
                body,
                "degree={} splineflags={} knots=[{}] ctrl=[{}] weights=[{}]",
                s.degree,
                bits,
                knots.join(","),
                ctrl.join(";"),
                weights.join(",")
            )
            .ok()?;
            "Spline"
        }
        // The recording lowers MTEXT to a Text record: the render protocol has
        // no multi-line record, and the adapter emits the insertion point,
        // height and rotation the same way it does for TEXT.
        EntityType::MText(t) => {
            let o = Ocs::new(&t.normal);
            let (x, y, z) = o.to_world(
                t.insertion_point.x,
                t.insertion_point.y,
                t.insertion_point.z,
            );
            let (x, y, z) = at.point(x, y, z);
            write!(
                body,
                "p=[{}] h={:.6} rot={:.6} value=\"{}\"",
                p3(x, y, z),
                t.height * at.scale_hint(),
                t.rotation,
                t.value
            )
            .ok()?;
            "Text"
        }
        // A heavy 2D polyline. Its vertices carry a full Vector3 but only x
        // and y are in the OCS plane; z on a 2D vertex is not the elevation.
        EntityType::Polyline2D(p2) => {
            let o = Ocs::new(&p2.normal);
            let pts: Vec<String> = p2
                .vertices
                .iter()
                .map(|v| {
                    let (x, y, z) = o.to_world(v.location.x, v.location.y, p2.elevation);
                    p3(x, y, z)
                })
                .collect();
            let bulges: Vec<String> = p2
                .vertices
                .iter()
                .map(|v| format!("{:.6}", v.bulge))
                .collect();
            let any = bulges.iter().any(|b| b != "0.000000");
            write!(
                body,
                "n={} closed={} pts=[{}] bulges=[{}] normal=[{}]",
                pts.len(),
                p2.flags.is_closed() as u8,
                pts.join(";"),
                if any { bulges.join(",") } else { String::new() },
                v3(&p2.normal)
            )
            .ok()?;
            "Polyline"
        }
        // A true 3D polyline. Its vertices are already world coordinates, so
        // no arbitrary-axis transform applies and it carries no bulges.
        EntityType::Polyline3D(p3d) => {
            let pts: Vec<String> = p3d.vertices.iter().map(|v| v3(&v.position)).collect();
            write!(
                body,
                "n={} closed={} pts=[{}] bulges=[] normal=[{}]",
                pts.len(),
                p3d.flags.closed as u8,
                pts.join(";"),
                v3(&p3d.normal)
            )
            .ok()?;
            "Polyline"
        }
        _ => return None,
    };
    Some(Record {
        kind: kind.into(),
        handle,
        flags: p.from_block as u32,
        body,
    })
}

/// The transform an INSERT applies to the entities of the block it names.
///
/// A 3x3 linear part and a translation, composed parent-first so a nested
/// insertion lands where the outer one puts it. Kept explicit rather than
/// pulled from a matrix crate because the only operations needed are compose
/// and apply, and a dependency here would be a dependency in the crate's dev
/// graph for the sake of six lines.
#[derive(Clone, Copy)]
pub struct Xform {
    m: [[f64; 3]; 3],
    t: [f64; 3],
}

impl Xform {
    pub fn identity() -> Self {
        Xform {
            m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            t: [0.0, 0.0, 0.0],
        }
    }

    /// The transform one insertion contributes: scale, then rotate about Z,
    /// then translate to the insertion point. That order is the one DXF
    /// defines, and swapping the first two is the classic way to get a
    /// mirrored insert subtly wrong.
    fn of_insert(i: &acadrust::entities::Insert) -> Self {
        let (c, s) = (i.rotation.cos(), i.rotation.sin());
        let (sx, sy, sz) = (i.x_scale(), i.y_scale(), i.z_scale());
        Xform {
            m: [
                [c * sx, -s * sy, 0.0],
                [s * sx, c * sy, 0.0],
                [0.0, 0.0, sz],
            ],
            t: [i.insert_point.x, i.insert_point.y, i.insert_point.z],
        }
    }

    fn then(self, inner: Xform) -> Self {
        let mut m = [[0.0; 3]; 3];
        for r in 0..3 {
            for c in 0..3 {
                m[r][c] = (0..3).map(|k| self.m[r][k] * inner.m[k][c]).sum();
            }
        }
        let mut t = [0.0; 3];
        for r in 0..3 {
            t[r] = (0..3).map(|k| self.m[r][k] * inner.t[k]).sum::<f64>() + self.t[r];
        }
        Xform { m, t }
    }

    fn point(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        (
            self.m[0][0] * x + self.m[0][1] * y + self.m[0][2] * z + self.t[0],
            self.m[1][0] * x + self.m[1][1] * y + self.m[1][2] * z + self.t[1],
            self.m[2][0] * x + self.m[2][1] * y + self.m[2][2] * z + self.t[2],
        )
    }

    /// True when this scales every axis alike. A circle under a non-uniform
    /// transform is an ellipse, and the recording emits a NonUniform warning
    /// rather than a distorted circle, so the two cannot be compared there.
    fn uniform(&self) -> bool {
        let sx = (self.m[0][0].powi(2) + self.m[1][0].powi(2)).sqrt();
        let sy = (self.m[0][1].powi(2) + self.m[1][1].powi(2)).sqrt();
        (sx - sy).abs() < 1e-9
    }

    fn scale_hint(&self) -> f64 {
        (self.m[0][0].powi(2) + self.m[1][0].powi(2)).sqrt()
    }

    fn is_identity(&self) -> bool {
        self.t == [0.0, 0.0, 0.0]
            && self.m[0][0] == 1.0
            && self.m[1][1] == 1.0
            && self.m[2][2] == 1.0
            && self.m[0][1] == 0.0
            && self.m[1][0] == 0.0
    }
}

/// One entity, with the transform that places it and whether it came from a
/// block. The flags bit is the recording's own, so it is compared rather than
/// stripped once expansion exists.
pub struct Placed<'a> {
    pub entity: &'a EntityType,
    pub at: Xform,
    pub from_block: bool,
}

/// Model space, with every insertion expanded.
///
/// `model_space_entities` rather than `entities`: the latter includes
/// block-definition geometry, so iterating it both emits a block's contents at
/// the origin AND emits them again through each insertion.
pub fn expand(doc: &acadrust::document::CadDocument) -> Result<Vec<Placed<'_>>, String> {
    let mut out = Vec::new();
    let mut stack: Vec<(&EntityType, Xform, bool, usize)> = doc
        .model_space_entities()
        .map(|e| (e, Xform::identity(), false, 0usize))
        .collect();
    stack.reverse();

    while let Some((e, at, from_block, depth)) = stack.pop() {
        if depth > 16 {
            return Err("block nesting deeper than 16".into());
        }
        match e {
            EntityType::Insert(i) => {
                let inner = at.then(Xform::of_insert(i));
                let members: Vec<&EntityType> = doc.entities_in_block(&i.block_name).collect();
                if members.is_empty() {
                    // An unresolved block, an xref most likely. The recording
                    // has its own answer for those and it is not geometry.
                    return Err(format!("block `{}` resolves to nothing", i.block_name));
                }
                for m in members.into_iter().rev() {
                    stack.push((m, inner, true, depth + 1));
                }
            }
            _ => out.push(Placed {
                entity: e,
                at,
                from_block,
            }),
        }
    }
    Ok(out)
}

pub fn dump_fixture(dwg: &Path, expectation: &Path) -> Verdict {
    let doc = match DwgReader::from_file(dwg).and_then(|mut r| r.read()) {
        Ok(d) => d,
        Err(e) => {
            let m = format!("{e}");
            return Verdict::OpenFailed(m.chars().take(80).collect());
        }
    };

    let placed = match expand(&doc) {
        Ok(p) => p,
        Err(e) => return Verdict::Uncompared(e),
    };

    for p in &placed {
        if let Some(why) = unrendered(p.entity) {
            return Verdict::Uncompared(why.to_string());
        }
        // A non-uniform transform over a curved primitive is a disagreement
        // about representation rather than about geometry: the recording warns
        // instead of emitting a distorted circle. Skipping the whole fixture
        // keeps that out of the record counts.
        if !p.at.uniform()
            && matches!(
                p.entity,
                EntityType::Circle(_) | EntityType::Arc(_) | EntityType::Ellipse(_)
            )
        {
            return Verdict::Uncompared(
                "a curved primitive under a non-uniform insertion; the recording warns rather than distorting it".into(),
            );
        }
    }

    let mut got: Vec<Record> = Vec::new();
    for p in &placed {
        if matches!(p.entity, EntityType::Viewport(_)) {
            continue; // the recording carries no viewport record
        }
        got.extend(render(p));
    }

    let text = match std::fs::read_to_string(expectation) {
        Ok(t) => t,
        Err(e) => return Verdict::OpenFailed(format!("expectation: {e}")),
    };
    let want: Vec<Record> = text.lines().filter_map(Record::parse).collect();

    if got.len() != want.len() {
        let kinds = |rs: &[Record]| {
            let mut v: Vec<String> = rs.iter().map(|r| r.kind.clone()).collect();
            v.sort();
            v.dedup();
            v.join(",")
        };
        return Verdict::Count {
            got: got.len(),
            want: want.len(),
            note: format!("acadrust [{}] vs recording [{}]", kinds(&got), kinds(&want)),
        };
    }

    let mut n = 0;
    let mut first = String::new();
    for (g, w) in got.iter().zip(want.iter()) {
        if g != w {
            n += 1;
            if first.is_empty() {
                first = format!(
                    "first at handle={}: acadrust `{} flags={} {}` vs recording `{} flags={} {}`",
                    w.handle, g.kind, g.flags, g.body, w.kind, w.flags, w.body
                );
            }
        }
    }
    if n == 0 {
        Verdict::Match
    } else {
        Verdict::Diff { n, first }
    }
}
