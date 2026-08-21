# rigidity-ui

**A viewer for [`rigidity`](../rigidity): register two point clouds and see
which degrees of freedom the geometry actually determined.**

The command line prints that answer as six lines of σ values. Six lines is
the right output for a script and the wrong output for a person — nobody
reads `ρ=[+0.00 +1.00 +0.00]` and pictures a corridor sliding along itself.
This application exists to make the ambiguous direction visible.

```
cargo run --release -- target.ply source.ply
```

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
| drag | orbit · right-drag pan · wheel dolly |

Files can also be dropped on the window. Anything that is not a parameter
lives in `⌘K` and nowhere else — there is no menu bar and no toolbar.

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

## State

Stage one is finished bar packaging: rendering, the pipeline, registration,
the null-space demonstration and the polish. `PLAN.md` has the rest —
a small opinionated CloudCompare next, and after it many scans with a
degeneracy-weighted pose graph, which is the part no other tool does.

## License

MIT or Apache-2.0, at your option — the same as `rigidity`.
