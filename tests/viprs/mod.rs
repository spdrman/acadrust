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

/// One scalar, with negative zero collapsed. A sign flip or a transform
/// produces it readily and it formats as "-0.000000" against the recording's
/// "0.000000".
fn f6(v: f64) -> String {
    format!("{:.6}", if v.abs() < 5e-7 { 0.0 } else { v })
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

/// An in-plane angle carried through a transform.
///
/// The angle is measured in the plane the entity's normal defines, so the
/// linear part alone cannot move it: the plane moves too. The direction at
/// that angle is built in the old basis, carried across, and read back in the
/// new one. Using only the transform's XY block, which is the obvious
/// shortcut, is right for an entity in the world XY plane and nonsense for one
/// that is not: it collapsed g13_ocs_rotated's arc onto a single angle.
fn angle_through(at: &Xform, normal: &acadrust::types::Vector3, a: f64) -> f64 {
    let o = Ocs::new(normal);
    let (c, si) = (a.cos(), a.sin());
    let d = (
        o.ax[0] * c + o.ay[0] * si,
        o.ax[1] * c + o.ay[1] * si,
        o.ax[2] * c + o.ay[2] * si,
    );
    let d2 = at.direction(d);
    let n2 = at.direction((normal.x, normal.y, normal.z));
    let o2 = Ocs::new(&acadrust::types::Vector3::new(n2.0, n2.1, n2.2));
    let x = d2.0 * o2.ax[0] + d2.1 * o2.ax[1] + d2.2 * o2.ax[2];
    let y = d2.0 * o2.ay[0] + d2.1 * o2.ay[1] + d2.2 * o2.ay[2];
    let r = y.atan2(x);
    if r < -1e-12 {
        r + std::f64::consts::TAU
    } else {
        r
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
fn render(doc: &acadrust::document::CadDocument, p: &Placed) -> Vec<Record> {
    // An ATTRIB reached through its insertion rather than through the
    // document. It is not an `EntityType` anywhere a reference can be taken
    // from — the DWG reader hangs it off the INSERT and puts nothing in the
    // entity table — so it travels beside the entity and is answered here,
    // before the entity's own kind is looked at.
    if let Some(a) = p.attribute {
        let mut body = String::new();
        if text_body(
            &mut body,
            &p.at,
            &a.normal,
            &a.insertion_point,
            a.height,
            a.rotation,
            &a.value,
        )
        .is_none()
        {
            return Vec::new();
        }
        return vec![Record {
            kind: "Text".into(),
            handle: format!("{:X}", a.common.handle.value()),
            flags: u32::from(p.from_block),
            body,
        }];
    }

    let handle = format!("{:X}", p.entity.common().handle.value());
    let at = &p.at;
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
            let w1 =
                p.at.point(f.first_corner.x, f.first_corner.y, f.first_corner.z);
            let w2 =
                p.at.point(f.second_corner.x, f.second_corner.y, f.second_corner.z);
            let w3 =
                p.at.point(f.third_corner.x, f.third_corner.y, f.third_corner.z);
            let e1 = [w2.0 - w1.0, w2.1 - w1.1, w2.2 - w1.2];
            let e2 = [w3.0 - w1.0, w3.1 - w1.1, w3.2 - w1.2];
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
        // LEADER: the vertex run, and nothing else. The arrowhead is a glyph
        // the dimension style names and is not in the drawing, which the
        // recording says in an ARROWHEAD_NOT_DRAWN warning rather than by
        // inventing one.
        EntityType::Leader(l) => {
            // A spline-fit leader's curve is not in the file as a curve, and
            // the recording declines to tessellate it: a polyline through the
            // fit points would be a tessellation by another name. So this
            // declines too rather than inventing the same curve differently.
            if matches!(l.path_type, acadrust::entities::LeaderPathType::Spline) {
                return Vec::new();
            }
            let pts: Vec<String> = l
                .vertices
                .iter()
                .map(|v| {
                    let (x, y, z) = p.at.point(v.x, v.y, v.z);
                    p3(x, y, z)
                })
                .collect();
            if pts.len() < 2 {
                return Vec::new();
            }
            let n = p.at.direction((l.normal.x, l.normal.y, l.normal.z));
            let mut body = String::new();
            if write!(
                body,
                "n={} closed=0 pts=[{}] bulges=[] normal=[{}]",
                pts.len(),
                pts.join(";"),
                p3(n.0, n.1, n.2)
            )
            .is_err()
            {
                return Vec::new();
            }
            vec![Record {
                kind: "Polyline".into(),
                handle,
                flags: p.from_block as u32,
                body,
            }]
        }
        // WIPEOUT: the clip boundary, as a closed polygon. The boundary is in
        // the unit square of the image's own frame, so it is mapped through
        // the insertion point and the u/v vectors rather than used directly.
        EntityType::Wipeout(w) => {
            let ip = &w.insertion_point;
            let (u, v) = (&w.u_vector, &w.v_vector);
            // The boundary is in the image's own frame, whose origin is the
            // top left and whose V axis runs DOWN. So u takes c.x + 0.5 and v
            // takes 0.5 - c.y, and reading v the same way as u mirrors every
            // boundary about the frame's middle, which on a symmetric one
            // looks entirely correct.
            let map = |cx: f64, cy: f64| {
                let (fu, fv) = (cx + 0.5, 0.5 - cy);
                let x = ip.x + fu * u.x + fv * v.x;
                let y = ip.y + fu * u.y + fv * v.y;
                let z = ip.z + fu * u.z + fv * v.z;
                p.at.point(x, y, z)
            };
            let cv = &w.clip_boundary_vertices;
            let world: Vec<(f64, f64, f64)> = if cv.len() == 2 {
                // A rectangular clip stores two opposite corners, not four.
                let (a, b) = (&cv[0], &cv[1]);
                vec![map(a.x, a.y), map(b.x, a.y), map(b.x, b.y), map(a.x, b.y)]
            } else {
                cv.iter().map(|c| map(c.x, c.y)).collect()
            };
            if world.len() < 3 {
                return Vec::new();
            }
            let pts: Vec<String> = world.iter().map(|w| p3(w.0, w.1, w.2)).collect();
            // The plane of the PLACED boundary, not the local frame's carried
            // across. A normal does not transform by the linear part: under a
            // reflection that gives the opposite sign, which is exactly the
            // case a mirrored insertion produces.
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
            let nn = norm(cross(e1, e2));
            let n = (nn[0], nn[1], nn[2]);
            let mut body = String::new();
            if write!(
                body,
                "n={} closed=1 pts=[{}] bulges=[] normal=[{}]",
                pts.len(),
                pts.join(";"),
                p3(n.0, n.1, n.2)
            )
            .is_err()
            {
                return Vec::new();
            }
            vec![Record {
                kind: "Polygon".into(),
                handle,
                flags: p.from_block as u32,
                body,
            }]
        }
        // HATCH lowers to its boundary loops. A loop made only of straight
        // edges is a closed polygon and becomes one record; a loop carrying a
        // curve cannot be expressed that way, and the recording says so in a
        // HATCH_LOOP_NOT_POLYGON warning and lets the edges follow as records
        // of their own. A hatch with no loop at all is pattern only and draws
        // nothing.
        EntityType::Hatch(hx) => {
            let o = Ocs::new(&hx.normal);
            let mut out = Vec::new();
            for path in &hx.paths {
                let straight = path.edges.iter().all(|e| {
                    matches!(
                        e,
                        acadrust::entities::BoundaryEdge::Line(_)
                            | acadrust::entities::BoundaryEdge::Polyline(_)
                    )
                });
                if !straight || path.edges.is_empty() {
                    // Its edges follow as their own records. This harness does
                    // not synthesise those, so the fixture reports a count
                    // difference rather than a silently short polygon.
                    continue;
                }
                let mut world: Vec<(f64, f64, f64)> = Vec::new();
                for e in &path.edges {
                    match e {
                        acadrust::entities::BoundaryEdge::Line(l) => {
                            let (x, y, z) = o.to_world(l.start.x, l.start.y, hx.elevation);
                            world.push(p.at.point(x, y, z));
                        }
                        acadrust::entities::BoundaryEdge::Polyline(pv) => {
                            for v in &pv.vertices {
                                let (x, y, z) = o.to_world(v.x, v.y, hx.elevation);
                                world.push(p.at.point(x, y, z));
                            }
                        }
                        _ => {}
                    }
                }
                if world.len() < 3 {
                    continue;
                }
                let pts: Vec<String> = world.iter().map(|w| p3(w.0, w.1, w.2)).collect();
                let n = p.at.direction((hx.normal.x, hx.normal.y, hx.normal.z));
                let mut body = String::new();
                if write!(
                    body,
                    "n={} closed=1 pts=[{}] bulges=[] normal=[{}]",
                    pts.len(),
                    pts.join(";"),
                    p3(n.0, n.1, n.2)
                )
                .is_err()
                {
                    continue;
                }
                out.push(Record {
                    kind: "Polygon".into(),
                    handle: handle.clone(),
                    flags: p.from_block as u32,
                    body,
                });
            }
            out
        }
        // MLINE, which is a path plus a style and lowers to one Polyline per
        // element of that style. This is the one lowering here that needs a
        // lookup, and the reason it is done rather than declined is that the
        // lookup's target is in the file: the style's element offsets, the
        // justification, the scale factor, and per vertex the position, the
        // segment direction and the joint's miter bisector.
        //
        // The formula is the reference implementation's, which took it from
        // the ODA specification and checked it two independent ways against
        // the same real_AC1032 in this corpus
        // (`native/Adapter/Flatten.MLine.cs`):
        //
        //     reference = 0 | max(offsets) | min(offsets)   by justification
        //     effective = (offset - reference) * scale_factor
        //     per vertex: D = unit(direction), M = unit(miter),
        //                 N = unit(normal x D), t = effective / dot(M, N),
        //                 point = position + M * t
        //
        // Dividing by dot(M, N) is what makes a bend meet itself: at a joint
        // the miter is longer than the offset by the secant of half the turn,
        // and offsetting each segment along its own perpendicular instead
        // leaves a gap at every corner. The vertices are world coordinates
        // already, so they are not lifted through the entity's plane; the
        // record still names that plane, the way a Polyline3D does.
        //
        // Emitting the centre path alone would be the defect the reference
        // implementation's comment names: a Polyline record with a plausible
        // point count, real coordinates, and no field saying it is the half
        // that did not need the lookup. So either every element crosses or
        // none does.
        EntityType::MLine(m) => {
            let offsets = mline_offsets(doc, m);
            if offsets.is_empty() || m.vertices.len() < 2 {
                // What the recording answers with is a warning, and this
                // harness compares geometry only, so no record is agreement.
                return Vec::new();
            }
            let reference = match m.justification {
                acadrust::entities::MLineJustification::Zero => 0.0,
                acadrust::entities::MLineJustification::Top => {
                    offsets.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                }
                acadrust::entities::MLineJustification::Bottom => {
                    offsets.iter().copied().fold(f64::INFINITY, f64::min)
                }
            };
            let normal = norm([m.normal.x, m.normal.y, m.normal.z]);
            // One denominator per vertex, measured once rather than once per
            // element, and all of them before anything is emitted: a vertex
            // no element can be placed at refuses the whole entity rather
            // than half of it, which is what the recording does.
            let mut denominators = Vec::with_capacity(m.vertices.len());
            for v in &m.vertices {
                let d = norm([v.direction.x, v.direction.y, v.direction.z]);
                let side = norm(cross(normal, d));
                let miter = norm([v.miter.x, v.miter.y, v.miter.z]);
                let denom = miter[0] * side[0] + miter[1] * side[1] + miter[2] * side[2];
                if denom.abs() < 1e-12 {
                    return Vec::new();
                }
                denominators.push(denom);
            }
            let closed = m.flags.contains(acadrust::entities::MLineFlags::CLOSED);
            let n = at.direction((m.normal.x, m.normal.y, m.normal.z));
            let mut out = Vec::with_capacity(offsets.len());
            for offset in &offsets {
                let effective = (offset - reference) * m.scale_factor;
                let pts: Vec<String> = m
                    .vertices
                    .iter()
                    .zip(&denominators)
                    .map(|(v, denom)| {
                        let miter = norm([v.miter.x, v.miter.y, v.miter.z]);
                        let t = effective / denom;
                        let (x, y, z) = at.point(
                            v.position.x + miter[0] * t,
                            v.position.y + miter[1] * t,
                            v.position.z + miter[2] * t,
                        );
                        p3(x, y, z)
                    })
                    .collect();
                let mut body = String::new();
                if write!(
                    body,
                    "n={} closed={} pts=[{}] bulges=[] normal=[{}]",
                    pts.len(),
                    closed as u8,
                    pts.join(";"),
                    p3(n.0, n.1, n.2)
                )
                .is_err()
                {
                    return Vec::new();
                }
                // Every element under the entity's own handle, which is the
                // MESH precedent: several records for one entity, consecutive.
                out.push(Record {
                    kind: "Polyline".into(),
                    handle: handle.clone(),
                    flags: u32::from(p.from_block),
                    body,
                });
            }
            out
        }
        _ => render_one(p).into_iter().collect(),
    }
}

/// An MLINE's element offsets, in style order.
///
/// The handle is the one the entity holds; the name is the fallback, because a
/// DXF-sourced document carries the style by name where a DWG carries it by
/// handle. An empty answer is "no offsets", which the caller turns into no
/// records rather than into the centre path.
fn mline_offsets(doc: &acadrust::document::CadDocument, m: &acadrust::entities::MLine) -> Vec<f64> {
    let by_handle = m.style_handle.and_then(|h| match doc.objects.get(&h) {
        Some(acadrust::objects::ObjectType::MLineStyle(s)) => Some(s),
        _ => None,
    });
    let style = by_handle.or_else(|| {
        doc.objects.values().find_map(|o| match o {
            acadrust::objects::ObjectType::MLineStyle(s) if s.name == m.style_name => Some(s),
            _ => None,
        })
    });
    style.map_or_else(Vec::new, |s| s.elements.iter().map(|e| e.offset).collect())
}

/// Record 10's body, shared by the four kinds that lower to it.
///
/// TEXT, MTEXT, ATTRIB and ATTDEF all cross as one Text record, and over
/// there they come out of one arm rather than four: ATTRIB and ATTDEF derive
/// from TextEntity, and MTEXT sits beside it writing the same three fields
/// (`native/Adapter/Flattener.cs:1088-1134`). One body here keeps the four
/// from drifting apart a field at a time.
///
/// The insertion point is lifted out of the plane the normal names, the way an
/// arc's centre is. The rotation is NOT carried through the transform, which
/// looks like an omission and is not: record 10 has no normal slot, so there
/// is nowhere to tell a consumer which plane the angle is measured in, and the
/// recording emits the in-plane angle for the same reason.
fn text_body(
    body: &mut String,
    at: &Xform,
    normal: &acadrust::types::Vector3,
    insertion_point: &acadrust::types::Vector3,
    height: f64,
    rotation: f64,
    value: &str,
) -> Option<()> {
    let o = Ocs::new(normal);
    let (x, y, z) = o.to_world(insertion_point.x, insertion_point.y, insertion_point.z);
    let (x, y, z) = at.point(x, y, z);
    write!(
        body,
        "p=[{}] h={:.6} rot={:.6} value={}",
        p3(x, y, z),
        height * at.scale_hint(),
        rotation,
        quote(value)
    )
    .ok()
}

/// A string the way the dump writes one.
///
/// The rule is not ours and is not a choice: it is `Q` in libviprs-dep's
/// `tests/fixtures/gen/CanonicalDump.cs:52`, and every dump on disk was
/// written through it. Quote, backslash, tab, CR and LF are escaped, a
/// control character below 0x20 becomes `\uXXXX` in lowercase hex, and
/// everything else crosses verbatim, so a drawing's own UTF-8 stays UTF-8.
///
/// Printing the value raw instead reads as a match on plain ASCII and falls
/// apart on the rest: real_AC1018's multi-line MTEXT broke its own record in
/// two, which the comparison then read as a missing record and an unexpected
/// one rather than as a text difference.
///
/// There is a sibling `Quote` in that repository's `Harness.cs` that also
/// escapes everything above 0x7E, and it is the WRONG one to copy: it writes
/// the scenario captures, not the record dumps. g11_codepage is what says so,
/// because its recorded value is `ÄÖÜ-O-??` rather than six escapes.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Below 0x20 is one UTF-16 unit either way, so the C# `x4` of a
            // char and this are the same four digits.
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
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
            // A non-uniform transform earns a NON_UNIFORM_BLOCK_SCALE warning
            // over there AND the record is still emitted, with the radius
            // taking the x scale. So this emits it too rather than declining.
            let o = Ocs::new(&c.normal);
            let (x, y, z) = o.to_world(c.center.x, c.center.y, c.center.z);
            let (x, y, z) = at.point(x, y, z);
            let n = at.direction((c.normal.x, c.normal.y, c.normal.z));
            write!(
                body,
                "c=[{}] r={:.6} normal=[({:.6},{:.6},{:.6})]",
                p3(x, y, z),
                c.radius * at.scale_hint(),
                n.0,
                n.1,
                n.2
            )
            .ok()?;
            "Circle"
        }
        EntityType::Arc(a) => {
            let o = Ocs::new(&a.normal);
            let (x, y, z) = o.to_world(a.center.x, a.center.y, a.center.z);
            let (x, y, z) = at.point(x, y, z);
            // A reflection reverses the sweep, so the transformed endpoints
            // change places as well as moving.
            let (mut b0, mut b1) = (
                angle_through(at, &a.normal, a.start_angle),
                angle_through(at, &a.normal, a.end_angle),
            );
            if at.mirrored() {
                std::mem::swap(&mut b0, &mut b1);
            }
            let n = at.direction((a.normal.x, a.normal.y, a.normal.z));
            write!(
                body,
                "c=[{}] r={:.6} a0={:.6} a1={:.6} normal=[({:.6},{:.6},{:.6})]",
                p3(x, y, z),
                a.radius * at.scale_hint(),
                b0,
                b1,
                n.0,
                n.1,
                n.2
            )
            .ok()?;
            "Arc"
        }
        EntityType::Ellipse(el) => {
            // An ellipse's centre and major axis are world coordinates, so
            // neither takes an OCS lift; the axis is a displacement and keeps
            // its length through the linear part. A reflection reverses the
            // parameter sweep, so the two parameters change places and sign,
            // and they are NOT wrapped into [0, tau): the recording prints the
            // negative.
            let c = at.point(el.center.x, el.center.y, el.center.z);
            let maj = at.linear((el.major_axis.x, el.major_axis.y, el.major_axis.z));
            let n = at.direction((el.normal.x, el.normal.y, el.normal.z));
            let (p0, p1) = if at.mirrored() {
                (-el.end_parameter, -el.start_parameter)
            } else {
                (el.start_parameter, el.end_parameter)
            };
            write!(
                body,
                "c=[{}] major=[{}] ratio={} p0={} p1={} normal=[{}]",
                p3(c.0, c.1, c.2),
                p3(maj.0, maj.1, maj.2),
                f6(el.minor_axis_ratio),
                f6(p0),
                f6(p1),
                p3(n.0, n.1, n.2)
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
            let flip = if at.mirrored() { -1.0 } else { 1.0 };
            let bulges: Vec<String> = pl.vertices.iter().map(|v| f6(v.bulge * flip)).collect();
            let any = bulges.iter().any(|b| b != "0.000000");
            write!(
                body,
                "n={} closed={} pts=[{}] bulges=[{}] normal=[{}]",
                pts.len(),
                pl.is_closed as u8,
                pts.join(";"),
                if any { bulges.join(",") } else { String::new() },
                {
                    let n = at.direction((pl.normal.x, pl.normal.y, pl.normal.z));
                    p3(n.0, n.1, n.2)
                }
            )
            .ok()?;
            "Polyline"
        }
        EntityType::Text(t) => {
            text_body(
                &mut body,
                at,
                &t.normal,
                &t.insertion_point,
                t.height,
                t.rotation,
                &t.value,
            )?;
            "Text"
        }
        // ATTRIB and ATTDEF both derive from TextEntity in ACadSharp, so the
        // reference implementation's text arm carries them rather than
        // needing one of their own (`native/Adapter/Flattener.cs:1106`), and
        // they cross as record 10 like any other text. The string is the one
        // the drawing shows: an instance's value, a definition's default.
        //
        // Nine of real_AC1018's Text records are ATTDEFs, and they were the
        // whole of that fixture's Text gap that was not the table below.
        EntityType::AttributeEntity(a) => {
            text_body(
                &mut body,
                at,
                &a.normal,
                &a.insertion_point,
                a.height,
                a.rotation,
                &a.value,
            )?;
            "Text"
        }
        EntityType::AttributeDefinition(a) => {
            text_body(
                &mut body,
                at,
                &a.normal,
                &a.insertion_point,
                a.height,
                a.rotation,
                &a.default_value,
            )?;
            "Text"
        }
        EntityType::Spline(s) => {
            let knots: Vec<String> = s.knots.iter().map(|k| f6(*k)).collect();
            let ctrl: Vec<String> = s.control_points.iter().map(v3).collect();
            let weights: Vec<String> = s.weights.iter().map(|w| f6(*w)).collect();
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
            text_body(
                &mut body,
                at,
                &t.normal,
                &t.insertion_point,
                t.height,
                t.rotation,
                &t.value,
            )?;
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
                    // The vertex's own third coordinate, not the polyline's
                    // elevation. g13_ocs_plane holds one of each at elevation
                    // 2: the LwPolyline's points land at z -2 and this one's
                    // at 0, so the two fields are not interchangeable.
                    let (x, y, z) = o.to_world(v.location.x, v.location.y, v.location.z);
                    p3(x, y, z)
                })
                .collect();
            let bulges: Vec<String> = p2.vertices.iter().map(|v| f6(v.bulge)).collect();
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
    /// then place the result in the plane the insertion's own extrusion
    /// defines, then translate to the insertion point. That order is the one
    /// DXF defines, and swapping the first two is the classic way to get a
    /// mirrored insert subtly wrong.
    ///
    /// The extrusion is the part that is easy to miss, because it is the
    /// identity on every insertion that sits in the world XY plane, which is
    /// most of them. An INSERT carries its own normal, its insertion point is
    /// measured in the plane that normal defines, and so is the block content
    /// it places. Ignoring it puts g13_ocs_rotated's circle at +3 instead of
    /// -3 and leaves its plane reading as the world's.
    fn of_insert(i: &acadrust::entities::Insert) -> Self {
        let (c, s) = (i.rotation.cos(), i.rotation.sin());
        let (sx, sy, sz) = (i.x_scale(), i.y_scale(), i.z_scale());
        let local = Xform {
            m: [
                [c * sx, -s * sy, 0.0],
                [s * sx, c * sy, 0.0],
                [0.0, 0.0, sz],
            ],
            t: [i.insert_point.x, i.insert_point.y, i.insert_point.z],
        };
        let o = Ocs::new(&i.normal);
        let basis = Xform {
            m: [
                [o.ax[0], o.ay[0], o.az[0]],
                [o.ax[1], o.ay[1], o.az[1]],
                [o.ax[2], o.ay[2], o.az[2]],
            ],
            t: [0.0, 0.0, 0.0],
        };
        basis.then(local)
    }

    /// The transform a TABLE contributes to the block it caches.
    ///
    /// A TABLE is an INSERT over there: `TableEntity` derives from `Insert`
    /// and the flattener dispatches on that base type
    /// (`native/Adapter/Flattener.cs:514`), so a table's drawn cell borders
    /// and cell text arrive out of the anonymous block at DXF 343 with the
    /// block-provenance flag set. acadrust models a TABLE as its own entity
    /// carrying the rows and the styles, so the placement an INSERT gets for
    /// free has to be built here.
    ///
    /// Rotation is not a field on it. DXF 11 is the horizontal direction
    /// vector, and the angle that makes in the plane the normal names is what
    /// an INSERT spells as its rotation. There is no scale: a table's cache is
    /// written in the coordinates the table is drawn at.
    fn of_table(t: &acadrust::entities::Table) -> Self {
        let o = Ocs::new(&t.normal);
        let h = &t.horizontal_direction;
        let dx = h.x * o.ax[0] + h.y * o.ax[1] + h.z * o.ax[2];
        let dy = h.x * o.ay[0] + h.y * o.ay[1] + h.z * o.ay[2];
        let len = (dx * dx + dy * dy).sqrt();
        // A zero horizontal direction is a table nothing said the rotation
        // of, which is the unrotated case rather than a malformed one.
        let (c, s) = if len < 1e-12 {
            (1.0, 0.0)
        } else {
            (dx / len, dy / len)
        };
        let local = Xform {
            m: [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
            t: [
                t.insertion_point.x,
                t.insertion_point.y,
                t.insertion_point.z,
            ],
        };
        let basis = Xform {
            m: [
                [o.ax[0], o.ay[0], o.az[0]],
                [o.ax[1], o.ay[1], o.az[1]],
                [o.ax[2], o.ay[2], o.az[2]],
            ],
            t: [0.0, 0.0, 0.0],
        };
        basis.then(local)
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

    /// Negative when the transform reflects. A reflection reverses what
    /// counter-clockwise means, which is what decides a bulge's sign, an arc's
    /// direction and a face's winding.
    fn mirrored(&self) -> bool {
        let m = &self.m;
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        det < 0.0
    }

    /// A direction through the linear part, renormalised. The record's normal
    /// is the plane its angles and bulges are measured in, so it has to follow
    /// the transform rather than be copied off the entity.
    fn direction(&self, v: (f64, f64, f64)) -> (f64, f64, f64) {
        let n = norm([
            self.m[0][0] * v.0 + self.m[0][1] * v.1 + self.m[0][2] * v.2,
            self.m[1][0] * v.0 + self.m[1][1] * v.1 + self.m[1][2] * v.2,
            self.m[2][0] * v.0 + self.m[2][1] * v.1 + self.m[2][2] * v.2,
        ]);
        (n[0], n[1], n[2])
    }

    /// Where an in-plane angle ends up. The point at that angle on the unit
    /// circle is carried through the linear part and read back as an angle,
    /// which handles rotation and reflection without either being special.
    fn angle(&self, a: f64) -> f64 {
        let (x, y) = (a.cos(), a.sin());
        let tx = self.m[0][0] * x + self.m[0][1] * y;
        let ty = self.m[1][0] * x + self.m[1][1] * y;
        let r = ty.atan2(tx);
        if r < 0.0 {
            r + std::f64::consts::TAU
        } else {
            r
        }
    }

    /// A vector through the linear part only, keeping its length. A major
    /// axis is a displacement rather than a direction, so it scales.
    fn linear(&self, v: (f64, f64, f64)) -> (f64, f64, f64) {
        (
            self.m[0][0] * v.0 + self.m[0][1] * v.1 + self.m[0][2] * v.2,
            self.m[1][0] * v.0 + self.m[1][1] * v.1 + self.m[1][2] * v.2,
            self.m[2][0] * v.0 + self.m[2][1] * v.1 + self.m[2][2] * v.2,
        )
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
    /// Set when what is placed is an ATTRIB the insertion carries rather than
    /// the entity itself, in which case `entity` is the INSERT it came off.
    ///
    /// It needs a channel of its own because the DWG reader hands an ATTRIB
    /// back inside `Insert::attributes` and puts nothing in the entity table,
    /// so there is no `&EntityType` to point at. The reference implementation
    /// has the same shape for the same reason: its `Pending` carries either an
    /// entity or a finished record (`native/Adapter/Flattener.cs:719`).
    pub attribute: Option<&'a acadrust::entities::AttributeEntity>,
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
            // A DIMENSION is a composite whose drawn geometry lives in an
            // anonymous block, with its own insertion scale and rotation. So
            // it expands exactly the way an INSERT does rather than needing a
            // second mechanism, and its contents carry the block flag for the
            // same reason.
            EntityType::Dimension(d) => {
                let local = Xform {
                    m: [
                        [
                            d.base().insertion_rotation.cos() * d.base().insertion_scale.x,
                            -d.base().insertion_rotation.sin() * d.base().insertion_scale.y,
                            0.0,
                        ],
                        [
                            d.base().insertion_rotation.sin() * d.base().insertion_scale.x,
                            d.base().insertion_rotation.cos() * d.base().insertion_scale.y,
                            0.0,
                        ],
                        [0.0, 0.0, d.base().insertion_scale.z],
                    ],
                    t: [0.0, 0.0, 0.0],
                };
                let inner = at.then(local);
                // A nested INSERT inside a dimension block is NOT walked, and
                // that is the reference implementation's decision rather than
                // an omission here: "a dimension block is generated geometry,
                // not a user block, and walking it as a block would hand it a
                // second depth budget" (`native/Adapter/Flattener.cs:1447`).
                // The blocks this skips are the terminators: real_AC1018
                // places `_BoxBlank` twice and `_ArchTick` twice inside `*D8`
                // and `*D4`, which is 12 records the recording does not have
                // and this used to emit.
                let members: Vec<&EntityType> = doc
                    .entities_in_block(&d.base().block_name)
                    .filter(|m| !matches!(m, EntityType::Insert(_)))
                    .collect();
                for m in members.into_iter().rev() {
                    stack.push((m, inner, true, depth + 1));
                }
            }
            EntityType::Insert(i) => {
                // An ATTRIB belongs to the insertion rather than to the block,
                // and it is already placed in the frame the insertion sits in,
                // so it goes out under the PARENT transform. Using `inner`
                // would transform a point that has already been transformed.
                // The reference implementation hands them out first, ahead of
                // the block's own entities (`native/Adapter/Flattener.cs:719`),
                // and going straight into `out` here is that order: what the
                // stack holds is popped after this.
                //
                // real_AC1018's are the four the comparison was missing:
                // `MyBlock`'s one value and `my_block_v2`'s three.
                for a in &i.attributes {
                    out.push(Placed {
                        entity: e,
                        attribute: Some(a),
                        at,
                        from_block,
                    });
                }

                let inner = at.then(Xform::of_insert(i));
                let members: Vec<&EntityType> = doc.entities_in_block(&i.block_name).collect();
                // An unresolved block, an xref most likely. The recording
                // answers with an UNRESOLVED_BLOCK warning and no geometry, so
                // contributing nothing here is agreeing with it, not skipping.
                for m in members.into_iter().rev() {
                    stack.push((m, inner, true, depth + 1));
                }
            }
            // A TABLE caches what it draws in an anonymous block, and over
            // there it is expanded because `TableEntity` derives from
            // `Insert`. Reaching that block is 56 of real_AC1018's 380
            // records: 31 cell-border lines, 24 cell texts and the one
            // background polygon, none of which exist on this side otherwise.
            // The rows and cells the entity itself carries are the table's
            // data rather than its picture, so nothing is synthesised from
            // them here: the picture is the block.
            EntityType::Table(t) => {
                // The DWG names the cache block by the handle at DXF 343 and
                // the entity's own `block_name` comes through empty, so the
                // name has to come out of the block table. Taking the name
                // when there is one keeps a DXF-sourced table working, where
                // the reverse holds.
                let name = if t.block_name.is_empty() {
                    t.block_record_handle.and_then(|h| {
                        doc.block_records
                            .iter()
                            .find(|br| br.handle == h)
                            .map(|br| br.name.clone())
                    })
                } else {
                    Some(t.block_name.clone())
                };
                // No cache block is a table nothing has drawn yet. The
                // recording has no records for one either, so contributing
                // nothing is agreeing with it.
                if let Some(name) = name {
                    let inner = at.then(Xform::of_table(t));
                    let members: Vec<&EntityType> = doc.entities_in_block(&name).collect();
                    for m in members.into_iter().rev() {
                        stack.push((m, inner, true, depth + 1));
                    }
                }
            }
            _ => out.push(Placed {
                entity: e,
                attribute: None,
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
        // A hatch loop carrying a curve is emitted over there as a warning
        // plus the loop's edges as records of their own. This harness does not
        // synthesise those, and emitting the straight loops alone would report
        // its own omission as a count difference against acadrust.
        if let EntityType::Hatch(hx) = p.entity {
            if hx.paths.iter().any(|path| {
                path.edges.is_empty()
                    || !path.edges.iter().all(|e| {
                        matches!(
                            e,
                            acadrust::entities::BoundaryEdge::Line(_)
                                | acadrust::entities::BoundaryEdge::Polyline(_)
                        )
                    })
            }) {
                return Verdict::Uncompared(
                    "a hatch loop carries a curve; the recording emits its edges as records and this does not"
                        .into(),
                );
            }
        }
    }

    let mut got: Vec<Record> = Vec::new();
    for p in &placed {
        if matches!(p.entity, EntityType::Viewport(_)) {
            continue; // the recording carries no viewport record
        }
        got.extend(render(&doc, p));
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
