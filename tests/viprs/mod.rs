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
    format!("({:.6},{:.6},{:.6})", x, y, z)
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
        let body = rest
            .split_whitespace()
            .filter(|t| !t.starts_with("handle=") && !t.starts_with("flags="))
            .collect::<Vec<_>>()
            .join(" ");
        Some(Record { kind, handle, body })
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
        EntityType::Insert(_) => Some("INSERT: the recording expands blocks, this does not"),
        EntityType::Hatch(_) => Some("HATCH: the recording lowers a hatch to boundary polygons"),
        EntityType::Dimension(_) => Some("DIMENSION: the recording expands the dimension block"),
        _ => None,
    }
}

fn render(e: &EntityType) -> Option<Record> {
    let handle = format!("{:X}", e.common().handle.value());
    let mut body = String::new();
    let kind = match e {
        EntityType::Line(l) => {
            write!(body, "pts=[{};{}]", v3(&l.start), v3(&l.end)).ok()?;
            "Line"
        }
        EntityType::Circle(c) => {
            let o = Ocs::new(&c.normal);
            let (x, y, z) = o.to_world(c.center.x, c.center.y, c.center.z);
            write!(
                body,
                "c=[{}] r={:.6} normal=[{}]",
                p3(x, y, z),
                c.radius,
                v3(&c.normal)
            )
            .ok()?;
            "Circle"
        }
        EntityType::Arc(a) => {
            let o = Ocs::new(&a.normal);
            let (x, y, z) = o.to_world(a.center.x, a.center.y, a.center.z);
            write!(
                body,
                "c=[{}] r={:.6} a0={:.6} a1={:.6} normal=[{}]",
                p3(x, y, z),
                a.radius,
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
        EntityType::LwPolyline(p) => {
            let o = Ocs::new(&p.normal);
            let pts: Vec<String> = p
                .vertices
                .iter()
                .map(|v| {
                    let (x, y, z) = o.to_world(v.location.x, v.location.y, p.elevation);
                    p3(x, y, z)
                })
                .collect();
            let bulges: Vec<String> = p
                .vertices
                .iter()
                .map(|v| format!("{:.6}", v.bulge))
                .collect();
            let any = bulges.iter().any(|b| b != "0.000000");
            write!(
                body,
                "n={} closed={} pts=[{}] bulges=[{}] normal=[{}]",
                pts.len(),
                p.is_closed as u8,
                pts.join(";"),
                if any { bulges.join(",") } else { String::new() },
                v3(&p.normal)
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
            write!(
                body,
                "p=[{}] h={:.6} rot={:.6} value=\"{}\"",
                p3(x, y, z),
                t.height,
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
            write!(
                body,
                "p=[{}] h={:.6} rot={:.6} value=\"{}\"",
                p3(x, y, z),
                t.height,
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
        body,
    })
}

pub fn dump_fixture(dwg: &Path, expectation: &Path) -> Verdict {
    let doc = match DwgReader::from_file(dwg).and_then(|mut r| r.read()) {
        Ok(d) => d,
        Err(e) => {
            let m = format!("{e}");
            return Verdict::OpenFailed(m.chars().take(80).collect());
        }
    };

    for e in doc.entities() {
        if let Some(why) = unrendered(e) {
            return Verdict::Uncompared(why.to_string());
        }
    }

    let mut got: Vec<Record> = Vec::new();
    for e in doc.entities() {
        if matches!(e, EntityType::Viewport(_)) {
            continue; // the recording carries no viewport record
        }
        if let Some(r) = render(e) {
            got.push(r);
        }
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
                    "first at handle={}: acadrust `{} {}` vs recording `{} {}`",
                    w.handle, g.kind, g.body, w.kind, w.body
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
