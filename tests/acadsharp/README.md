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
| `MATCH` | 39 |
| `DIFF` | 1 |
| `COUNT` | 3 |
| `UNCOMPARED` | 1 |
| no recording | 6 |

The harness models the DWG format's own semantics: the arbitrary axis algorithm, block
expansion including an insertion's own extrusion, reflection, non-uniform scale, and
dimension blocks. It was bootstrapped rather than trusted: block expansion reproduces
`g13_insert` exactly, rotation and the `flags` bit included, against a recording made by
the real ACadSharp.

## The two real differences

**Extrusion normals are not normalized.** acadrust returns `(1,2,2)` where ACadSharp
returns `(0.333,0.667,0.667)`, the same direction over its length. Every coordinate beside
it agrees to six decimals, so the arbitrary-axis maths underneath is right and only the
returned vector is raw. Visible on `g13_solid`.

**Non-finite geometry passes straight through.** `g13_nan_bulge` holds a NaN bulge and an
Infinity. acadrust emits both; ACadSharp's adapter emits neither, because its wire contract
promises finite values. A difference in contract rather than in parsing, and worth
recording because a consumer porting off ACadSharp inherits the filtering job without being
told.

## Where the harness deliberately stops

Two lowerings are the adapter's product decisions rather than the format's semantics, and
this harness does not reimplement them:

- **MLINE** (`g13_mline`, 0 of 21 records). The offsets live in a style table, and the
  joints are mitred. How to mitre is a choice, not a fact about the file.
- **A hatch loop carrying a curve** (`g13_hatch`). ACadSharp emits a warning and then the
  loop's edges as records of their own. Which curve becomes which record is likewise a
  choice. Straight-edged loops ARE compared, and match.

That line is the whole reason this report is worth reading. Past it the harness would be a
second implementation of the thing it is measuring, and every difference it found could be
its own. Where it cannot express a fixture it says `UNCOMPARED` rather than emitting fewer
records, because a short count reads as the other side's defect.

`real_AC1018` and `real_AC1032` sit at 318 of 380 records for the same reason. They are
measured rather than skipped, and the 62-record gap is not yet attributed between those two
lowerings and anything else.

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
