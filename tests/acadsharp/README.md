# Differential conformance against ACadSharp

ACadSharp is the reference implementation this crate is measured against. It is
a .NET library, so running it here is not on the table, and the comparison is
against a recording of it instead: [`libviprs-dep`][dep] drives the real
ACadSharp through a flattener and commits, per fixture, the canonical record
dump it produced.

[dep]: https://github.com/libviprs/libviprs-dep

```bash
scripts/fetch-acadsharp-corpus.sh
ACADSHARP_CORPUS=target/acadsharp-corpus cargo test --test acadsharp_conformance

# the human-readable version, one line per fixture
ACADSHARP_CORPUS=target/acadsharp-corpus \
  cargo test --test acadsharp_conformance -- --ignored report --nocapture
```

With no corpus the test skips rather than fails. Fetching it is a network call
and a crate's suite should not need one to go green.

## Where it stands

52 fixtures, of which 46 carry a recording, read from libviprs-dep at the pinned commit.

| Verdict | Count |
| --- | --- |
| `MATCH` | 28 |
| `DIFF` | 6 |
| `COUNT` | 4 |
| `UNCOMPARED` | 7 |
| no recording | 6 |

Block expansion is implemented here, and it was validated before being trusted: it
reproduces `g13_insert` exactly, rotation and the `flags` bit included, against a
recording made by the real ACadSharp. That bootstrap is what makes the expander usable on
the fixtures whose answer is not already known.

## The two real differences

**Extrusion normals are not normalized.** On `g13_ocs_plane` acadrust reports
`normal=[(1.000000,2.000000,2.000000)]` where ACadSharp reports
`normal=[(0.333333,0.666667,0.666667)]`, the same direction over its length. The circle's
centre agrees to every decimal place, so the arbitrary-axis maths underneath is right and
only the returned vector is raw. It shows up twice, on `g13_ocs_plane` and `g13_solid`.

**Non-finite geometry passes straight through.** `g13_nan_bulge` holds a polyline with a
NaN bulge and one with an Infinity. acadrust reads and emits both; ACadSharp's adapter
emits neither and says its wire contract promises finite values. A difference in contract
rather than in parsing, and worth recording because a consumer porting off ACadSharp
inherits the filtering job without being told.

## What is this harness's own gap, not acadrust's

Seven fixtures, and they are listed rather than buried because a conformance report that
cannot separate its own omissions from the thing it measures is worth nothing:

- **No renderer yet**: `g13_leader`, `g13_mline`, `g13_wipeout`. ACadSharp lowers these
  onto `Polyline` and `Polygon` records; this does not render them at all.
- **Mirror handling**: `g13_mirrored_bulge`, `g13_ocs_mirror`, `g13_face3d`. A mirrored
  insertion reverses what counter-clockwise means, so a bulge's sign, an arc's direction
  and a face's winding all follow it. This applies the point transform and not that.
- **OCS composed with an insertion**: `g13_ocs_rotated`. The normal has to follow the
  transform rather than be copied off the entity.

## What is not compared at all

Seven `UNCOMPARED`: two dimension fixtures and three hatch files, where the recording
expands a composite this harness does not; one non-uniform insertion of a curved
primitive, where ACadSharp warns rather than distorting a circle and the two are not
comparable by construction; and one unresolved xref.

## Why a baseline instead of asserting equality

`BASELINE.tsv` records the per-fixture verdict and the test asserts against it.
acadrust is not ACadSharp and this going green was never the goal: a
conformance test that is red on arrival is one nobody can use twice, because
the second run tells you exactly what the first did.

A fixture that gets worse fails and names what moved. A fixture that gets
better also fails, and asks you to re-record. Both are the point.

```bash
ACADSHARP_CORPUS=... cargo test --test acadsharp_conformance -- --ignored record_baseline
```

## The pin

`CORPUS.pin` names the libviprs-dep commit the corpus is read from. It is
pinned rather than floating for the reason the corpus is committed over there
in the first place: a DWG header carries creation and update timestamps, so a
regenerated fixture is a different file with a different digest, and a
comparison against whatever `main` holds today cannot be reproduced.

## Licensing

The corpus is MIT. The generated fixtures are libviprs-dep's own, written by
ACadSharp's `DwgWriter`; `real_AC1018.dwg` and `real_AC1032.dwg` are ACadSharp's
own sample drawings. Nothing from the corpus is vendored into this repository,
which keeps provenance in the one place that records it.
