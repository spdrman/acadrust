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

52 fixtures, of which 45 carry a record dump, read from libviprs-dep at the pinned commit.

| Verdict | Count |
| --- | --- |
| `MATCH` | 39 |
| `DIFF` | 4 |
| `COUNT` | 1 |
| `UNCOMPARED` | 1 |
| no recording | 7 |

The seven with no dump are not a gap in the corpus and nothing here should try to fill
it: five are checked by the scenario capture (`expectations/g13_scenarios.json`) and two by
the benchmark capture (`benchmarks/amplification.json`), which is why libviprs-dep's own
`recorded_fixtures()` reads all three artefacts rather than the dumps alone. They are
limit, refusal and block-amplification fixtures, so a geometry dump is not what records
them. The 45 are the whole of the geometry oracle.

The harness models the DWG format's own semantics: the arbitrary axis algorithm, block
expansion including an insertion's own extrusion, reflection, non-uniform scale, dimension
blocks, a TABLE's cached block, an insertion's ATTRIBs, and MLINE's element offsets. It was
bootstrapped rather than trusted: block expansion reproduces `g13_insert` exactly, rotation
and the `flags` bit included, against a recording made by the real ACadSharp.

Both real drawings now produce all 380 records and `g13_mline` all 21, so what is left is
four differences in content rather than in count.

## The four real differences

**Extrusion normals are not normalized.** acadrust returns `(1,2,2)` where ACadSharp
returns `(0.333,0.667,0.667)`, the same direction over its length. Every coordinate beside
it agrees to six decimals, so the arbitrary-axis maths underneath is right and only the
returned vector is raw. Visible on `g13_solid`.

**Non-finite geometry passes straight through.** `g13_nan_bulge` holds a NaN bulge and an
Infinity. acadrust emits both; ACadSharp's adapter emits neither, because its wire contract
promises finite values. A difference in contract rather than in parsing, and worth
recording because a consumer porting off ACadSharp inherits the filtering job without being
told.

**MTEXT inline escapes are decoded.** acadrust hands back `94°` and `∅45,6` where ACadSharp
hands back `94\U+00B0` and `\U+220545,6`, the escape the drawing stores. One more of the
same kind collapses a run of eleven spaces to one. Neither side is wrong about the file and
both are defensible, but a consumer that pattern-matches on the text gets a different answer
from each, so the difference is recorded rather than normalised away here. `real_AC1018` is
`DIFF:4` and `real_AC1032` `DIFF:2` on exactly these.

**The DWG MLINE closed flag is dropped.** `g13_mline`'s fourth multiline is written with
`MLineFlags.Has | MLineFlags.Closed` and the recording says `closed=1` for its three
element polylines; acadrust reads the entity back with `MLineFlags(HAS_VERTICES)`, bits `1`,
so this emits `closed=0`. Every coordinate on all 21 records agrees, which is what makes it
a flag that is not read rather than a path that is wrong. `g13_mline` is `DIFF:3`.

## Where the harness deliberately stops

One lowering is the adapter's product decision rather than the format's semantics, and this
harness does not reimplement it:

- **A hatch loop carrying a curve** (`g13_hatch`). ACadSharp emits a warning and then the
  loop's edges as records of their own. Which curve becomes which record is a choice, not a
  fact about the file. Straight-edged loops ARE compared, and match.

That line is the whole reason this report is worth reading. Past it the harness would be a
second implementation of the thing it is measuring, and every difference it found could be
its own. Where it cannot express a fixture it says `UNCOMPARED` rather than emitting fewer
records, because a short count reads as the other side's defect.

MLINE used to sit beside it, on the grounds that mitring is a choice. That reading did not
survive libviprs-dep#110: the joint is not chosen, it is `t = effective / dot(miter, side)`
from the per-vertex miter bisector the file carries, which upstream took from the ODA
specification and confirmed against `real_AC1032` two independent ways. The style lookup it
needs resolves inside the same document. So it is reproduced here, and the one thing that
does not agree is the closed flag above.

Two matching decisions are worth naming because they look like omissions here:

- **A nested INSERT inside a dimension block is not walked**, because the reference
  implementation does not walk one: "a dimension block is generated geometry, not a user
  block". That skip is 12 records in each real drawing — `_BoxBlank` twice inside `*D8`,
  `_ArchTick` twice inside `*D4` — which this harness used to emit and the recording never
  had.
- **A TABLE is expanded as an insertion**, because `TableEntity` derives from `Insert`
  upstream and the flattener dispatches on that base type. That block is 56 of each real
  drawing's records: 31 cell borders, 24 cell texts and one background polygon.

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
