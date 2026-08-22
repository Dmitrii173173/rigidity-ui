<img width="2560" height="2140" alt="rigidity-ui-hero" src="https://github.com/user-attachments/assets/ad6d303a-fae8-4bf9-b2de-7802cbd7663c" />



# rigidity-ui

**A viewer for [`rigidity`](https://github.com/Dmitrii173173/rigidity): register two point clouds and see
which degrees of freedom the geometry actually determined.**

The command line prints that answer as six lines of σ values. Six lines is
the right output for a script and the wrong output for a person — nobody
reads `ρ=[+0.00 +1.00 +0.00]` and pictures a corridor sliding along itself.
This application exists to make the ambiguous direction visible.

```
cargo run --release -- target.ply source.ply
```

PLY, LAS, LAZ, E57, PCD and delimited text — `.txt` or `.csv` — go in, and
every one of them comes out again: **save a cloud to a file…** in `⌘K`,
where the extension you type chooses the format. A cloud saved as `.laz` is
compressed and one saved as `.e57` is not, and there is no second control
anywhere to say so.

Or open it with nothing and press the **corridor** demo: it builds a scene
whose null space is known analytically, and a copy of it displaced by a
known amount, so there is something to look at within a second of starting.

## What the screen says

**The spectrum** is six degrees of freedom, each with the spread the
geometry leaves in it, on a logarithmic axis crossed by two vertical lines —
the accuracy you asked for, and ten times it. A bar past the first line is
the finding. Colour repeats what position already says; nothing here
depends on telling teal from amber.

**Hover a row** and the cloud swings along that direction while every point
takes the colour of its own contribution to it. On the corridor, σ₁ lights
up the two walls and not the floor — the floor's normal is perpendicular to
that motion and does nothing to resist it — while σ₆, translation along the
corridor, lights up almost nothing at all. The same 101 mm of motion; a
fourteenfold difference in how much the image changes.

Both come from one identity. The Jacobian row of a point with normal `n`,
projected onto the i-th singular direction `ξᵢ = [ρ; φ]`, is

```
rowᵢ · ξᵢ  =  n·ρ + (p×n)·φ  =  n · (ρ + φ×p)  =  n · v
```

— the point's own motion under that direction, projected onto its own
normal. The swing is `v` integrated; the colour is `v` projected. Since
`σᵢ² = Σ w (n·v)²`, the colouring is the σ bar taken apart over the
geometry.

**The sliders are free.** Sensor noise, required tolerance and the
empirical correction move the threshold lines and recolour the bars within
the frame, with no computation behind them: the conditioning holds the
spectrum, and both `uncertainty` and `classify` are pure functions of six
stored numbers. "Is five millimetres acceptable here?" becomes a question
you answer by looking.

**There are no modes.** One cloud is a question about a surface and the
answer arrives unasked. A second makes it a registration.

## What it will not tell you

Whether the solver found the *right* minimum. Conditioning describes the
shape of the cost function around wherever it stopped; inside a wrong basin
the surfaces agree just as tightly and the report looks just as confident.
When the residual exceeds three times the sensor noise you stated, the
spectrum is drawn faint and says so. Believe the residual first.

## Keys

| | |
|---|---|
| `⌘K` | every command, by name |
| `⌘O` | open a cloud |
| `space` | run |
| `esc` | stop |
| `F` | fit the view |
| `M` | measure between two points |
| `⌘S` | save the project |
| `⌘G` | solve the survey |
| shift-drag | lasso: keep or delete what it encloses |
| drag | orbit · right-drag pan · wheel dolly |

Files can also be dropped on the window. Anything that is not a parameter
lives in `⌘K` and nowhere else — there is no menu bar and no toolbar.

## Projects

```
cargo run --release -- survey.rgp
```

A survey is more scans than anyone re-places by hand after closing a
window, so the scene can be written to a project: which files are in it,
where each one sits, and which two are playing source and target.

```
rigidity-project 1

scan  scans/station-00.laz
pose  1.0 0.0 0.0 0.0 1.0 0.0 0.0 0.0 1.0 0.0 0.0 0.0
voxel 0.03
role  target
shown yes
```

Text, so it can be read, diffed and repaired in an editor — the point of
the application is not trusting numbers you cannot see. Paths are relative
to the project where they can be, because a survey is a directory that gets
copied. A pose is the rotation matrix and the translation, twelve numbers
rather than the six of a twist, and that is the one decision here that is
not a matter of taste: a twist would round-trip through `exp` and `log` and
come back *near* where it started, and a project that drifts every time it
is saved cannot be used to measure drift. Twenty scans reopen at the same
bits, and there is a test that says so.

Each scan carries its own preparation, because a survey is not made of
clouds at one density: a station taken up against a wall wants a finer
voxel than one taken across a hall, and making the pair agree lets the
coarser of the two decide for both. The **prepare** sliders name the scan
they are about and a new scan starts at whatever they were last left at, so
setting them once and opening twenty scans still gives twenty scans that
agree. Only a scan tuned away from the default writes `voxel` and
`neighbours` into the project — a file that spells out values nobody chose
reads as though somebody did.

A registration is not written until it is **placed** — `⌘K`, *place the
source where the registration put it* — which moves the scan's own pose to
where the solver left it. Nothing on screen moves when you do: what changes
is whether the position survives the next run.

## The survey

A registration is a fact about two scans. A survey is many of them, and the
thing worth having is what they say together.

Register a pair, then **keep this registration as a survey edge** in `⌘K`.
The edge carries the weight its own conditioning justifies: the directions
the geometry determined, and *nothing at all* along the ones it did not. On
a corridor that is five of six, and the row in the **survey** panel says so.
Every other package in this space takes the registration's `JᵀWJ` at face
value, which is confident about the corridor's own axis and wrong.

**Solve the survey** — `⌘G` — moves every scan so the edges agree as well as
their weights say they can, and the status strip reports what it cost and
how far the worst scan moved. The first cloud in the scene is the anchor and
stays where it is; a pose graph fixes its nodes only up to a common rigid
motion, and some node has to be the survey's origin.

The edges are drawn in the viewport between the scans they join, amber where
a registration did not determine all six directions. They are saved with the
project, weight and all: an edge is a decision about a run that has since
been replaced, and rebuilding it would mean running that registration again.

On a synthetic loop with one leg down a corridor, this is worth a factor of
about 120 in the worst scan's drift — the measurement is in `rigidity`'s
`rigidity-graph`, and it is the reason this project exists.

### Closing the loop

The survey line under the heading counts the closures, and it is the number
worth reading. A survey walked as a chain — every scan registered against
the last — has none, and the panel says so in as many words: *nothing is
checked against anything*. That is not a warning about precision. A chain
is a tree; no two of its measurements are ever compared, so the solve
reaches zero residual at whatever answer the chain gives it, and every small
error made along the way is still in that answer.

Closing it is the same four gestures as any other edge — pick the two scans
that are next to each other in the room and far apart in the chain, run,
keep, solve. On a synthetic twelve-station loop that takes the worst
station from 361 mm out to 22 mm.

If the survey is in more than one piece, the solve says which scans are
joined to nothing that reaches the anchor, instead of failing to factorise
and leaving you to work out why.

### After the solve

Each edge gains two numbers: how far apart the solve left its ends, and how
hard it is pulling. The pair is the diagnosis and neither half is it alone.
An edge left half a metre apart that pulls at almost nothing is an edge
whose weight along that direction was removed — it could not see along
there, the survey settled by another path, and nothing is wrong. Half a
millimetre apart while pulling hard is a measurement losing an argument it
should be winning, and that is the one to go back and do again.

Under them, the **least certain stations**: how well the survey as a whole
knows where each scan is, worst first, in metres and degrees. That is the
spectrum's question one level up — not what a single registration
determined, but what all of them together did. A station nothing joins to
the anchor says so in words rather than being given a plausible number.

One measurement decided how this is computed, and it is worth knowing about
if you ever move it. A survey four million metres from zero has to be
conjugated onto itself before the solve: not because the poses are large —
every quantity the solve touches is a relative one, good to a nanometre —
but because `weighted_information` refers its directions to the coordinate
origin, where a milliradian of rotation carries four kilometres of
translation. The matrix conditions at 1e18 there against 1e2 at the survey,
and solving without moving it puts the answer 58 m out on a 20 m loop.

Twenty scans of a million points is seven times what the viewport can draw
at 60 fps, so each large scan keeps a coarse copy and the viewport draws
those while the camera is moving, going back to the full clouds a fifth of
a second after it stops. Twenty million points orbit at 120 fps instead of
8; the measurements and why this rather than an octree are in `PLAN.md` §8.

## Numbers you can check

Every screen that shows a number can hand back the `rigidity …` invocation
that produces it — **copy the command line** in `⌘K`, including the
`rigidity scene` lines when the clouds were generated here. That is not a
convenience: a viewer whose defaults have drifted from the command line's
prints numbers nobody can compare with anyone else's, and no user could
detect it. The test suite registers a generated pair through the viewer's
own path and requires the command line's output to contain the viewer's
report byte for byte.

## Building

The two repositories are cloned side by side. `rigidity` is on crates.io,
but this crate depends on it by *path*: stage three of the plan develops a
pose-graph crate upstream and calls it from here in the same sitting, and a
pinned version would put a publish between every two edits.

```
git clone https://github.com/Dmitrii173173/rigidity.git
git clone https://github.com/Dmitrii173173/rigidity-ui.git
cd rigidity-ui
```

```
cargo test
cargo clippy --all-targets -- -D warnings
```

Five dependencies, one toolchain, no system libraries. The pinned compiler
in `rust-toolchain.toml` matches `../rigidity`'s: this project is about
small singular values, and the two halves have to be built by the same
`rustc` or their numbers are not comparable.

`RIGIDITY_UI_BENCH=<seconds>` orbits the camera and prints the frame
distribution; `RIGIDITY_UI_SHOT=<path>` writes the window out. They exist
because frame times cannot tell a correct image from an empty one, and
several of the milestone gates are about what the image shows.

## A double-clickable application

```
./package/macos.sh
```

assembles `build/rigidity.app` — three files in two directories, written by
a thirty-line script rather than a build-time tool, and an icon generated by
`package/icon.py` so that it can be regenerated and argued with rather than
being a binary of unknown provenance. The bundle is unsigned: on any machine
that did not build it, the first launch needs a right-click and **Open**.

It declares no document types. Putting `.ply` in "Open With" would place the
application on the Dock's drop target, where files arrive as an Apple event
it does not answer — and a promise that does nothing is worse than no
promise. Files are dropped on the window.

## State

Stages one and two are finished: rendering, the pipeline, registration, the
null-space demonstration, and then the ten tools that make it somewhere you
can spend a day — many clouds, scalar fields, selection and geometry,
manual alignment, and the formats a survey arrives in. Stage three has
started: projects hold a survey's scans and their poses. What is left of it
is the part no other tool does — a pose graph whose edges are weighted by
each registration's own conditioning, so that an edge from a corridor
carries no weight along the corridor. `PLAN.md` has the whole of it.

## License

Dual-licensed: **AGPL-3.0-only**, or a commercial licence — the same as
`rigidity`, and not a separate decision, since this application links those
crates directly.

Free under the [AGPL](LICENSE) for students, research, personal use and
evaluation. Closed products and hosted services need the commercial
licence: [`LICENSING.md`](LICENSING.md).
