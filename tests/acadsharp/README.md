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

43 fixtures, of which 36 carry a recording.

| Verdict | Count | Means |
| --- | --- | --- |
| `MATCH` | 15 | every geometry record agreed, to 1e-6 |
| `DIFF:n` | 1 | same record count, n records disagreed |
| `COUNT:a/b` | 1 | acadrust produced a records, ACadSharp b |
| `UNCOMPARED` | 16 | the fixture needs flattening this harness does not do |
| no recording | 7 | benchmark and limit fixtures with no expectation committed |

`g13_scale_1x` is the one to look at before believing the rest: 128 records,
64 circles and 64 lines, every one of them matching to six decimal places.

## The two real differences

**Extrusion normals are not normalized.** On `g13_ocs_plane` acadrust reports
`normal=[(1.000000,2.000000,2.000000)]` where ACadSharp reports
`normal=[(0.333333,0.666667,0.666667)]`, which is the same direction divided by
its length. The centre of that circle agrees exactly, to every decimal place, so
the arbitrary-axis maths underneath is right and it is the vector handed back
that is raw. Anything treating the normal as a unit vector, which is what a
normal usually is, gets a wrong answer scaled by 3.

**Non-finite geometry passes straight through.** `g13_nan_bulge` holds a
polyline with a NaN bulge and one with an Infinity. acadrust reads both and
emits them. ACadSharp's adapter emits neither, and says why:

```
Warning code=NON_FINITE_GEOMETRY message="Polyline value 13 is NaN, and
docs/WIRE.md promises no geometry record carries a value that is not finite,
so this entity is not emitted"
```

That is a difference in contract rather than in parsing. The file really does
contain NaN, and acadrust never promised to filter it. It is recorded here
because a consumer porting off ACadSharp inherits the filtering job without
being told, and NaN reaching a renderer is not a failure that announces itself.

## What is not compared, and why that matters

16 fixtures are `UNCOMPARED`. Every one of them holds an INSERT, a HATCH or a
DIMENSION, and the recording lowers all three to primitives: a block is
expanded and transformed, a hatch becomes its boundary polygons, a dimension
becomes the contents of its dimension block.

This harness does none of that, deliberately. Writing a second flattener here
would mean every difference it found could be its own bug rather than
acadrust's, and a conformance report that cannot tell those apart is worse than
no report. So the fixtures that need one say so.

That leaves the honest summary: **on the geometry this harness can compare,
acadrust agrees with ACadSharp on 15 of 17 fixtures, and the two it does not
agree on are both narrow and both understood.** Block expansion, hatch
boundaries and dimension flattening are unmeasured, and they are the parts a
CAD renderer leans on hardest.

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
