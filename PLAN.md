# rigidity-ui — development plan

Plan version: 2026-08-21. Crate versions are the ones `Cargo.lock`
actually resolved: eframe/egui/egui-wgpu 0.36.1, wgpu 30.0.0, nalgebra
0.35.0. Everything is vendored in the local registry, and
`cargo build --offline` is green.

Companion documents: [`rigidity/README.md`](https://github.com/Dmitrii173173/rigidity/blob/master/README.md),
[`rigidity/PLAN.md`](https://github.com/Dmitrii173173/rigidity/blob/master/PLAN.md),
[`rigidity/MOTIVATION.md`](https://github.com/Dmitrii173173/rigidity/blob/master/MOTIVATION.md).

## Status

The work is in three stages. Stage 1 is the instrument — the thing no
other tool does. Stage 2 makes it somewhere you can actually spend a day.
Stage 3 is where the instrument pays off at scale. They are strictly
ordered: stage 2 without stage 1 is a worse CloudCompare, and stage 3
without stage 2 has nowhere to put its scans.

| Milestone | State | Note |
|---|---|---|
| **Stage 1 — the instrument** | | |
| M0 — Scaffold | **done** | window, wgpu callback, shell layout, theme |
| M1 — Cloud rendering | **done** | splats, orbit camera, EDL, 1.05 M at 120 fps |
| M2 — Pipeline and spectrum | **done** | jobs, cancel, σ panel, parity with the CLI |
| M3 — Registration | **done** | live ICP, timeline, residuals, basin warning |
| M4 — Null-space visualisation | **done** | hover to move, |n·v| colouring, demos |
| M5 — Polish | **done** | ⌘K, both themes measured, errors, README |
| M6 — Packaging | **done** | `.app` bundle, generated icon, CI green on three OSes |
| **Stage 2 — the workbench** | | |
| W1 — Many clouds | **done** | scene list, roles as chips, one buffer per cloud |
| W2 — Scalar fields | **done** | ramps, histogram, cloud-to-cloud distance |
| W3 — Selection and geometry | **done** | lasso, cross-section, measure, subsample |
| W4 — Manual alignment | **done** | point pairs, Kabsch upstream, the wrong basin escaped |
| W5 — Formats | **done** | LAS/LAZ, E57, PCD, export; one door for all of them |
| **Stage 3 — the survey** | | |
| S1 — Projects | **done** | poses bit-identical, parameters per scan, LOD decided and built |
| S2 — Pose graph | **done** | `rigidity-graph` upstream, gate 120×; edges, solve, panel, viewport, saved with the project |
| S3 — Loop closure | not started | manual first, detected later |
| S4 — Whole-survey view | not started | drift, residuals per edge, per-scan conditioning |
| **Upstream** | | |
| §7 — first four changes | **done** | landed in `../rigidity`; CLI output unchanged |
| §7 — items 5 and 6 | **done** | Kabsch landed with W4, E57 with W5 |
| §7 — item 7 | **done** | `rigidity-graph`, and an `SE(3)` Jacobian in the core |
| §7 — item 8 | not needed yet | serde; S1 shipped without it and the format is the better for it |

---

## 0. What this is, in one paragraph

`rigidity` answers a question no other registration tool answers: *which
degrees of freedom did the geometry actually determine?* The CLI prints that
answer as six lines of σ values. Six lines is the right output for a script
and the wrong output for a human — nobody looks at `ρ=[+0.00 +1.00 +0.00]`
and pictures a corridor sliding along itself. **This application exists to
make the ambiguous direction visible.** Loading, rendering and registering
are infrastructure in service of that one screen.

That is stage 1, and it stays the reason the application exists. Stages 2
and 3 follow from a plainer observation: nobody opens a diagnostic on its
own. The measurement happens inside a day's work — clouds get cropped,
compared, measured, aligned by hand when ICP lands in the wrong basin, and
eventually there are forty of them rather than two. A diagnostic that
forces the user out to CloudCompare and back for every one of those steps
is a diagnostic that does not get used.

So: **stage 2 is a small, opinionated CloudCompare** — the ten tools out of
its hundreds that are used daily, and no more. Not a clone; a workbench
good enough that the instrument is at hand. **Stage 3 is many scans**: a
project, a pose graph, loop closure. It belongs here rather than being
scope creep for one reason, and it is a strong one — see §8, stage 3.

The order is not negotiable. Stage 2 before stage 1 would be a worse
CloudCompare with nothing to offer; stage 3 before stage 2 would have
nowhere to put its scans. And the filter stays the same at every stage: a
feature belongs here if it **answers a question the CLI cannot**. Anything
that is only "the same thing, but with a button" stays in the CLI.

---

## 1. Principles that are not revisited

1. **The frame loop never blocks.** Every call into `rigidity` runs on a
   worker thread. If a stage takes 400 ms the UI still redraws at 60 fps and
   still responds to Esc.
2. **`rigidity` stays headless.** The UI is a consumer, not a co-owner. Four
   small upstream changes are requested in §7; none of them add a UI concept
   to the core, and none of them block M0–M2.
3. **One toolchain, no system libraries.** Same promise the core makes: no
   Node, no Python, no Qt, no CMake. `cargo run` on a clean machine.
4. **Numerical parity with the CLI.** The same input produces the same pose,
   the same RMSE and the same spectrum, bit for bit. Every screen that shows
   a number can emit the `rigidity …` command line that reproduces it, and a
   test asserts the two agree.
5. **`f32` stays `f32`.** `PointCloud::columns()` goes to the GPU as it is
   stored. No `f64` round trip in the render path, no second copy of the
   cloud living in the renderer's own format.
6. **Minimal chrome.** No menu bar, no icon toolbar, no floating tool
   windows. One inspector panel, one status strip, a command palette. A
   feature that needs new permanent chrome needs a stronger case than "it is
   convenient".
7. **No widget framework of our own.** If a screen cannot be built from what
   `egui` offers plus a custom paint call, the screen is wrong.

---

## 2. Stack

```toml
[dependencies]
# Path dependencies, and they stay. `rigidity` has a remote and is
# published — 0.1.1, all eight crates — so a version or git dependency
# would now resolve; §10 says why neither is taken.
#
# Two entries rather than four: §7.2 put the whole file-to-report sequence
# in `rigidity-pipeline`, which pulls io, spatial and core behind it. The
# viewer and the CLI now call the same code, so they cannot drift.
rigidity-core     = { path = "../rigidity/crates/rigidity-core" }
rigidity-pipeline = { path = "../rigidity/crates/rigidity-pipeline" }

# One dependency for the whole UI and GPU stack. eframe re-exports the
# `egui`, `egui_wgpu` and `wgpu` it was built against, so those three
# cannot drift out of step with each other or with us — which was the
# entire risk this section used to warn about.
eframe = "0.36"

# Vertex and uniform structs go to the GPU as bytes.
bytemuck = { version = "1.25", features = ["derive"] }

# Later, as the milestones need them:
#   nalgebra = { version = "0.35", features = ["convert-bytemuck"] }  # M1 camera
#     — or `rigidity_core::nalgebra` (§7.3) if no extra features are wanted
#   rfd = "0.17"        # M1 file dialogs
#   thiserror = "2.0"   # M2 engine errors
```

M1 needed even less than that. `nalgebra` is not declared here at all:
`rigidity_core::nalgebra` (§7.3) is the same crate, and `Matrix4::as_slice`
is already the column-major order a WGSL `mat4x4` expects, so no bytemuck
conversion and no feature flag were required. `rfd` came in for the file
dialog. The manifest is five dependencies.

**What stages 2 and 3 add — and how little it is.** Worth stating plainly,
because "a CloudCompare and a SLAM package" sounds like it should drag in a
dependency tree, and it does not:

| Need | Answer |
|---|---|
| Selection, cross-section, measurement, crop | Own code. Screen-space maths and a `KdTree` we already build. |
| Point picking | CPU ray against the kd-tree first; a GPU id buffer only if that proves too slow, which at one nearest-neighbour query per click it will not. |
| Colour ramps, histograms | Own code. A 1-D lookup texture and a vertex attribute. |
| Cloud-to-cloud distance | `rigidity-spatial`, unchanged. |
| Absolute orientation (W4) | A 3×3 SVD — `nalgebra` has it; the solver belongs upstream (§7). |
| LAS/LAZ, E57, PCD (W5) | `las` with its laz feature and the `e57` crate are already declared in `../rigidity`'s workspace; a PCD crate gets chosen at W5, not now. |
| Project files (S1) | `serde` + `toml`, one small pair, in the viewer only. The core stays serde-free. |
| Pose-graph solve (S2) | `nalgebra` dense to a few hundred poses. `faer` is the candidate if and when sparse is genuinely needed — decided against a measurement, not in advance. |
| Loop-closure descriptors (S3) | Own code. The published descriptors in this space are a page of arithmetic each; a crate would be a dependency for less code than reading its docs. |

The rule the table encodes: a dependency is taken when it does something we
cannot correctly do ourselves in comparable effort — file formats, linear
algebra, windowing — and not otherwise.

**Why this and not something else.**

| Alternative | Why not |
|---|---|
| Tauri / Electron + three.js | The data path is the product. Getting a million points into JS means serialising them — and `rigidity-core` has no `serde` at all. Adds a second toolchain to a project whose selling point is not needing one. |
| Bevy | An ECS and an asset pipeline for an application with two clouds and one panel. Its UI layer would end up being `bevy_egui` anyway. |
| iced | The most attractive default look of the Rust options, and `iced::widget::shader` can host a wgpu pass. But the 3D viewport *is* the application here, not an inset; `egui-wgpu`'s paint callback is the shorter, better-trodden path to it. Revisit if the panel work ever outgrows immediate mode. |
| Slint | Same viewport objection. Its royalty-free terms are also a second licence to reason about on top of this project's own AGPL/commercial split, for a UI layer `egui` already covers. |
| Rerun (already a dependency of `rigidity-viz`) | Excellent, and it stays — for *debugging* a registration run. It is a general viewer: it cannot put a slider next to a spectrum and re-classify on drag, which is §5's whole point. Use `rigidity-viz` when the question is "what did ICP do"; use this app when the question is "what is this answer worth". |
| glam for camera math | A second linear-algebra crate to avoid `nalgebra`'s f32 path, which is fast enough for four matrices per frame. `convert-bytemuck` closes the only real gap. |

**Toolchain.** `rust-toolchain.toml` is copied from `../rigidity` (1.97.1,
pinned deliberately: a floating compiler means floating small singular
values). Rustup resolves the file by walking *up* from the current
directory, so a sibling folder inherits nothing — the copy is required, not
cosmetic. The whole UI stack builds on 1.97.1; the risk in §10 is closed.

---

## 3. Layout

```
rigidity-ui/
  Cargo.toml              # own workspace, single crate for now
  rust-toolchain.toml     # copy of ../rigidity's pin
  PLAN.md
  README.md               # written at M5, not before
  src/                    # ✓ = exists after M0
    main.rs               # ✓ window bootstrap, nothing else
    app.rs                # ✓ layout, state machine, event pump
    theme.rs              # ✓ palette, type scale, spacing
    engine/
      mod.rs              # ✓ thread, channels, request identifiers
      job.rs              # ✓ Job / Event
      session.rs          # ✓ the pipeline call sequence, and the parity test
    bench.rs              # ✓ frame times and screenshots, for the gates
    commands.rs           # ✓ the ⌘K list
    field.rs              # ✓ a scalar per point, its range and its clamps
    histogram.rs          # ✓ the distribution and the two handles
    spectrum.rs           # ✓ the six σ rows and their threshold lines
    timeline.rs           # ✓ the iteration strip and its residual curve
    render/
      mod.rs              # ✓ pipelines, targets, callback
      camera.rs           # ✓ orbit / pan / dolly, fit-to-bounds
      cloud.wgsl          # ✓ vertex-pulled splats, model matrix, ramp
      composite.wgsl      # ✓ eye-dome lighting and the blit
    panels/               # when there are enough of them to be a directory;
      inspector.rs        # the inspector and the status strip are still
      status.rs           # sections of app.rs, and splitting two functions
```

`engine/` is still to be written, but its shape is now much smaller than
this plan first assumed: `rigidity-pipeline` (§7.2) already owns the call
sequence and the typed errors, so `engine/` is a worker thread, a job
queue and a cancellation flag — not a second copy of the pipeline.

`engine/` is written from the start as if it were a library crate with no
knowledge of `egui`: it is the candidate to be lifted into `rigidity` as
`rigidity-pipeline` (§7.2). Nothing in `engine/` may import `egui` or `wgpu`.

---

## 4. Architecture

**Two threads, three channels.**

```
  UI thread                          worker thread
  ─────────                          ─────────────
  egui frame loop        Job  ─────▶ session state
  render pass           ◀───── Event  (clouds, trees, normals)
  never blocks           Cancel ────▶ AtomicBool
```

`Job`: `Load{slot, path}`, `Prepare{slot, params}`, `Analyse{slot}`,
`Register{params}`, `Reclassify{criteria}`.
`Event`: `Stage{name, done, total}`, `Cloud{slot, Arc<PointCloud>}`,
`Iteration(IterationReport)`, `Analysis(Box<Analysis>)`, `Failed(EngineError)`.

Four consequences worth stating explicitly, because they shape everything
downstream:

1. **Clouds are shared, never copied.** `Arc<PointCloud>` crosses the
   channel; the renderer uploads once and keeps a generation counter.
2. **Pose changes cost 64 bytes.** During ICP the source cloud does not move
   in the vertex buffer — the model matrix does. A 200-iteration run writes
   one uniform per iteration and re-uploads nothing. This is why live ICP
   playback is affordable at a million points.
3. **Re-classification is free.** `Conditioning` stores the spectrum;
   `classify(&criteria)` and `uncertainty(σ)` are pure functions of the six
   stored singular values. Dragging the noise or tolerance slider does *not*
   re-run the SVD — it recolours six rows. The sliders can therefore be
   continuous, which is exactly the interaction the CLI cannot offer.
4. **Cancellation granularity is honest.** §7.1 landed, so Esc stops ICP
   at the next *accepted* iteration — a rejected step raises the damping
   and retries without reporting, so the answer can lag by a few
   assemblies. The status strip says what Esc actually did rather than
   claiming an instant stop.

**Coordinates.** `PointCloud` stores `f32` offsets from an `f64` origin. The
GPU gets the offsets untouched; the camera lives in the *target's* local
frame; the source's model matrix carries the origin difference, computed in
`f64` and narrowed once. Georeferenced clouds at 500 km from the origin
therefore render at millimetre fidelity, which is the whole reason the core
stores points that way.

---

## 5. The interface

```
┌────────────────────────────────────────────────────────┐
│ ⌘K                                                  ◐  │  28 px, draggable
├──────────────┬─────────────────────────────────────────┤
│ SOURCE       │                                         │
│ corridor_s…  │                                         │
│ 1 200 000    │                                         │
│ → 34 361     │                                         │
│              │                                         │
│ TARGET       │              viewport                   │
│ corridor_t…  │                                         │
│ 1 200 000    │                                         │
│ → 34 402     │                                         │
│              │                                         │
│ ─ SPECTRUM ─ │                                         │
│ σ₁ ████████  │                                         │
│ σ₂ ███████   │                                         │
│ σ₃ ██████    │                                         │
│ σ₄ █████     │                                         │
│ σ₅ ███       │                                         │
│ σ₆ ▌     LOW │  ← hover                                │
│              │                                         │
│ noise  ──○─  │                                         │
│ toler. ─○──  │                                         │
│ calib. ──○─  │                                         │
├──────────────┴─────────────────────────────────────────┤
│ ● registered · 50 it · rmse 1.20e-3 m · 34 361 corr.   │  24 px
└────────────────────────────────────────────────────────┘
```

**One piece of mathematics, shown three ways.** These are not three
features to prioritise against each other; they are one identity seen from
three sides, which is why the screen can carry all of them and stay quiet.

The Jacobian row of a point `p` with normal `n` is `[nᵀ | (p×n)ᵀ]`. Project
it onto the i-th singular direction `ξᵢ = [ρ; φ]`:

```
rowᵢ · ξᵢ  =  n·ρ + (p×n)·φ  =  n · (ρ + φ×p)  =  n · v
```

where `v` is the instantaneous velocity of `p` under the rigid motion `ξᵢ`.
**A point's contribution to σᵢ is the projection of its own motion onto its
own normal.** Since `σᵢ² = Σ w·(n·v)²`, the three views below are literally
the same number:

1. *Moving the cloud along `ξᵢ`* — integrate `v`. The corridor slides along
   itself and the image does not change.
2. *Colouring each point by `|n·v|`* — the same quantity per point. This
   decomposes the σ bar over the geometry: it shows which points hold that
   direction, and in a degenerate direction it shows that almost none do.
3. *Drawing `v` as arrows on a sample of points* — where the arrows lie flat
   along a surface, that surface contributes nothing; where they push into
   it, that is the constraint. Which is also the answer to "where should the
   next scan point", without having to phrase it as advice.

**The one interaction that matters.** Hovering σ₆ does not highlight a row.
It takes the null-space direction from `Conditioning::direction_in_world(5)`
and gently oscillates the source cloud along it, with an amplitude of a few
times the predicted spread. On the corridor scene the cloud slides along the
corridor and *nothing appears to change* — which is precisely what "this
degree of freedom is not determined" means, shown rather than asserted.
Hovering σ₁ does the same and the walls visibly tear apart. Release, and it
eases back to the registered pose in 160 ms.

**The tolerance is a line, not a label.** The spread axis is logarithmic —
in the README's own example the six values span 6.6·10⁻⁵ to 3·10⁻³ — and a
single vertical line across it marks `--tolerance`. Classification stops
being a word next to a bar and becomes geometry: the bars that cross the
line are the problem. Dragging the tolerance slider moves the line and
recolours the bars live, because `classify()` and `uncertainty()` are pure
functions of the six stored singular values and cost nothing to re-evaluate.
That interaction is the single cheapest thing on this screen and probably
the most useful: it turns "is 5 mm of spread acceptable?" from a judgement
into a look.

**The mode is inferred, never chosen.** One cloud loaded: the spectrum of
that surface — *if anything were registered against this, what would it
determine?* — which is a scan-planning tool in its own right and is cheaper
than registration, so it arrives first (M2). A second cloud loaded: the
registration appears. There are no tabs, no radio buttons and no mode
switch, because there is nothing a mode switch would tell the application
that the scene does not already say.

Every other decision follows from keeping that screen uncluttered:

- **Loading**: drag and drop onto the viewport, or ⌘O. No file browser panel.
- **Commands**: ⌘K palette. Run, cancel, reset view, fit, copy CLI line,
  swap source/target, load demo scene, toggle theme. Nothing that lives in
  the palette also gets a button.
- **Keyboard**: Space runs, Esc cancels, F fits, 1–4 switch colour mode,
  ⌘⇧C copies the reproducing command line.
- **Colour modes**: flat · height · residual · contribution to the hovered σ.
  The last one is the natural pair to the hover interaction: it shows *which
  points* are holding that direction down.
- **Iteration timeline**: appears only after a registration, as a thin scrub
  strip above the status line. Poses come from the `register_observed`
  callback; scrubbing is a uniform write. **Any pose on it can become the
  next starting pose** — `register_pair_observed` already takes an
  `initial`, so "scrub back to where it still looked right, change the
  parameters, run from there" costs one argument. It is most of the value of
  a manual alignment tool for none of its cost, and it stays useful after
  W4 adds the real one.
- **Export, three things**: the report as text, identical to what the CLI
  prints; the transformed source as PLY; and the command line that
  reproduces the numbers on screen. The last is not a convenience — §1
  requires that every screen showing a number can produce the `rigidity …`
  invocation that yields it.
- **RMSE is never far from the spectrum.** The README is explicit that
  conditioning cannot detect a wrong local minimum, and that on real data 11
  of 30 pairs converged to a wrong basin with perfectly healthy spectra. A UI
  that shows a confident spectrum without the residual next to it would be
  actively misleading. When RMSE is high relative to the stated sensor
  noise, the spectrum panel is marked unreliable — greyed, with the reason
  spelled out — rather than merely accompanied by a number. One comparison
  of two floats; without it the application lies confidently in exactly the
  case that matters most.
- **Demo scenes in the palette.** `rigidity-scenes` generates a corridor, a
  corner, a cylinder and four more with analytically known null spaces and
  no files involved. The application explains itself in ten seconds to
  someone who has not got a dataset to hand, at nearly zero cost.

**Visual language.** Dark by default, light supported, system-aware. Near
black `#0E0F11`, two surface steps above it, one accent. Separation by
spacing, never by borders. 4 px grid, 13 px UI text, tabular numerals in the
σ table so the columns line up. The observability ramp is amber → teal, not
red → green: three states must survive deuteranopia and both themes. Motion
is 120–160 ms ease-out on state changes and camera moves, and nowhere else.

---

## 6. Rendering

- **Splats by vertex pulling.** Four vertices per point, expanded in the
  vertex shader to a screen-space square, discarded outside the disc in the
  fragment shader. Point size in pixels, optionally attenuated by distance.
  `PrimitiveTopology::PointList` is one pixel per point and looks like noise.
- **No interleave at all.** `columns()` hands out three `&[f32]` slices and
  they become three vertex streams, one per axis. The plan originally
  budgeted one copy into a `Vec<[f32; 3]>`; the copy turned out to be
  avoidable, so coordinates now reach the GPU exactly as `PointCloud`
  stores them. The core's array-of-columns layout was chosen for filtering
  and SIMD, and it pays a third time here.
- **Reverse-Z `f32` depth**, so a scene spanning four orders of magnitude
  does not z-fight at the far end.
- **Eye-dome lighting** as a full-screen post pass over depth. Point clouds
  without normals are unreadable without it; with it, they read as surfaces.
  This is the single cheapest thing that separates "looks modern" from
  "looks like 2009".
- **Budget.** One million points is four million vertices — comfortable at
  60 fps on Apple Silicon. Above roughly five million, render the
  voxel-downsampled cloud during interaction and the full one when the
  camera is idle. Level of detail beyond that is decided at S1's gate, where
  the number of scans is known, rather than guessed at now.

What the later stages add to this list, and what they do not:

- **A model matrix per cloud** (W1). Already the design — a pose change is a
  uniform write, never a re-upload — so many clouds cost buffers, not passes.
- **A scalar attribute buffer and a 1-D ramp texture** (W2). One extra vertex
  attribute; the ramp and its clamps live in the uniform, so dragging a
  histogram handle is a 64-byte write and not a re-upload of anything.
- **Clipping planes** (W3) as a uniform the fragment shader discards against.
  A cross-section is then free to animate, which is what makes it usable.
- **Picking** (W3) on the CPU: unproject the click, query the kd-tree, done.
  No id buffer, no readback, no frame of latency.

---

## 7. Changes made upstream in `rigidity` — **done**

All four landed in `../rigidity` before M0, in one commit. None of them
adds a UI concept to the core, and all four were free: the crates are
unpublished, so nothing was a breaking change.

The CLI was rewritten onto the new pipeline crate and its output was
checked against a build of the previous commit — eight invocations across
all three subcommands and an error path, byte for byte identical including
exit codes. The full CI gate (fmt, clippy `-D warnings`, 98 tests, the
one-thread-versus-eight determinism run, rustdoc `-D warnings`) is green.

1. **Cancellation.** ✓ `register_observed`'s observer now returns
   `ControlFlow<()>` and a `Break` ends the run. Convergence is still
   decided first: whether the step met the thresholds is a fact about the
   step, not about the caller's patience, so a run that converges on the
   same iteration the caller cancels reports `converged: true` truthfully.
   The alternative — calling `register` repeatedly with
   `max_iterations = 1` — was rejected: it costs a second assembly per
   iteration and resets the Levenberg–Marquardt damping each call, which
   would have broken parity with the CLI.
2. **Hoist the pipeline out of the binary.** ✓ A new `rigidity-pipeline`
   crate, not `rigidity-cli/src/lib.rs` as this plan first proposed: a
   viewer must not pull `clap` into its dependency graph, and the workspace
   already draws its boundaries by concern (core = mathematics, spatial =
   index, io = files, pipeline = orchestration, cli = argv). It holds
   `prepare`/`prepare_cloud`, `register_pair`, `analyse_cloud`,
   `analyse_registration`, `transform_cloud`, the parameter structs, a
   `Progress`/`Stage` pair, and a `PipelineError` that replaced the CLI's
   `Result<_, String>` throughout. `main.rs` lost ninety lines.

   Two of its signatures were shaped by the viewer rather than by the CLI,
   and both earn their place: `prepare_cloud` takes a cloud already in
   memory, so changing the voxel size does not re-read the file and a
   generated scene needs no file at all; and `analyse_registration` takes a
   *pose* rather than an `IcpResult`, so the conditioning at iteration
   twelve of a recorded run is as askable as the conditioning at the end.
3. **`pub use nalgebra;` from `rigidity-core`.** ✓ The public API is full
   of `Vector3<f64>`, `Vector6<f64>` and `Matrix6<f64>`; a downstream crate
   that resolves a different `nalgebra` gets type errors that read as
   nonsense. `rigidity-pipeline` was the first beneficiary — it uses
   `rigidity_core::nalgebra` and needs no `nalgebra` dependency of its own.
4. **Progress for the non-ICP stages.** ✓ `voxel_downsample_observed`,
   `estimate_normals_observed` and `KdTree::build_observed`, each taking
   `FnMut(usize, usize)` and each called only from the calling thread, so a
   caller needs no atomics and sees monotonic counts.

   The units differ per stage and the documentation says so rather than
   pretending otherwise: normals report in points (the range is chunked at
   16 384, which changes neither values nor order, since points are
   independent), while downsampling and the kd-tree report in whole
   internal phases — four and two. `kiddo` builds its tree in one opaque
   call that is the larger half of the wait, and a bar that interpolated
   through it would be a bar that lies.

### What stages 2 and 3 need — 5 and 6 done, 7 and 8 outstanding

Listed so the boundary stays visible: the viewer contributes screens,
`rigidity` contributes mathematics. Anything whose test does not mention a
pixel belongs upstream. Two of these four were still hypothetical when this
section was written and were built as stage 2 reached them.

5. **Absolute orientation** (W4). ✓ Horn/Kabsch from corresponding point
   pairs: the SVD of a 3×3 correlation matrix, with the reflection case
   handled. It sits beside the Lie-group code in `rigidity-core::lie` and is
   testable against exactly the kind of analytical oracle the rest of that
   crate uses.
6. **E57** (W5) in `rigidity-io`. ✓ Landed with the rest of the formats,
   behind the one dispatcher that chooses a reader by extension. The want
   of test data that deferred it upstream was answered the way the other
   formats were: a round-trip test writes the fixture it then reads, twice,
   once at the origin and once at a UTM coordinate far enough out to catch
   an f32 that should have been an f64.
7. **The pose graph** (S2). ✓ `rigidity-graph`, beside `rigidity-core` as
   this section guessed, and the guess about where the difficulty would be
   was wrong in an instructive direction. The optimiser was the easy half.
   The hard halves were a Jacobian the core did not have — `SE(3)`'s, whose
   `Q` block needed two series coefficients with a threshold fifty times the
   module's usual — and a gate that at first proved nothing, because a
   perfectly degenerate synthetic corridor makes `JᵀWJ` exactly singular and
   the naive weighting free. The corridor in the gate now has a far end, two
   points wide, which is what a real one has.

   Worst-station drift over a loop with one slipped corridor leg: 7.32 mm
   naive, 0.061 mm weighted. The calibration is deliberately not in the
   crate — it arrives through `criteria.noise_sigma`, which is where this
   viewer's ×17 slider already puts it, so the screen and the graph cannot
   disagree.
8. **Optionally `serde` behind a feature** (S1). Not needed. S1 shipped
   holding paths, poses, roles, visibility and two preparation numbers, all
   of it written by hand in twenty lines, and the format is better for it:
   the pose is twelve numbers because bit-identity demanded a matrix rather
   than a twist, and a derive would have picked whatever `Se3`'s fields
   happened to be. The core stays serde-free.

---

## 8. Milestones

Each milestone has a gate. A gate is a thing that either passes or does not;
"looks fine" is not a gate.

### Stage 1 — the instrument

**M0 — Scaffold. Done.** Workspace, pinned toolchain, path dependencies
resolving against `../rigidity`, an `eframe` window with an `egui-wgpu`
paint callback drawing one triangle inside the viewport rect. The shell
layout and the palette came with it, because the triangle had to sit
somewhere and a viewport with no panels around it would have to be torn out
at M1 anyway.
*Gate: passed.* `cargo run` opens a window and holds it with no wgpu
validation output; `cargo clippy --all-targets -- -D warnings` is clean;
`cargo build --offline` succeeds.

What the triangle is for: it is drawn from a real vertex buffer through a
real uniform written every frame from a value the UI owns, and it is
divided by the viewport's aspect ratio. A resize that skews it, or a
rotation that stops, means the path from `app.rs` to a pixel is broken —
which is the only thing M0 can usefully assert. The window keeps the
native title bar; the frameless treatment is M5, where it can be looked at
rather than guessed at (eframe 0.36 exposes `WindowChromeMetrics` for
exactly that).

**M1 — Cloud rendering. Done.** Load a PLY off the frame loop, upload,
orbit camera, fit-to-bounds, splats, eye-dome lighting, point-size and
shading controls, drag and drop, and a path on the command line.
*Gate: passed.* A 1 050 000-point corridor, orbiting: median 8.33 ms, p95
8.60 ms, worst 16.9 ms over 712 frames. Read as a floor rather than a
measurement — 8.33 ms is this display's refresh interval, so the figure
says the renderer keeps up with a 120 Hz panel, not how much headroom is
left above it.

Two things arrived early and one is missing:

- **The worker thread**, which this plan put in M2. Principle §1 forbids a
  blocking frame loop and reading a million points takes long enough to
  break it, so `engine/` exists now — a thread, a job, an event and a
  repaint signal. The full queue, with cancellation, still belongs to M2.
- **`bench.rs`**, the measurement harness: `RIGIDITY_UI_BENCH=<seconds>`
  orbits the camera by itself and prints the frame distribution;
  `RIGIDITY_UI_SHOT=<path>` writes the window out as raw RGBA on the way
  down. It exists because frame times cannot tell a correct image from an
  empty one — a viewport that draws nothing is very fast — and three later
  gates are about what the image shows. It was worth its thirty lines
  immediately: it caught eye-dome lighting doing almost nothing at the
  strength and radius first chosen, which no timing would have shown.
- **The colour modes** are not here. M1 draws one flat colour and lets EDL
  do the work, which is enough to read a surface; height, residual and
  contribution belong with the data that gives them meaning.

**M2 — Pipeline and spectrum. Done.** A job vocabulary with identifiers,
progress by stage, cancellation, single-cloud `analyse`, and the σ panel
with live noise, tolerance and correction sliders.
*Gate: passed*, and stronger than this plan asked for. The test builds the
CLI, runs it on a generated corridor, runs the viewer's own path over the
same cloud, and requires the viewer's report to appear in the CLI's output
**byte for byte** — not the six values compared to some tolerance, the
whole printed report. The comparison is against `describe()` because that
is the artefact people quote at each other, and because it also covers the
condition number, the correspondence count and every label. Checked by
mutation: changing the CLI's voxel from 0.05 to 0.07 fails it.

The fixture is generated by `rigidity-scenes` rather than read from
`demo/`, which is gitignored upstream. A test that skips when its input is
missing is a test that passes for the wrong reason.

What the milestone settled beyond its brief:

- **Requests carry identifiers, and the engine finishes only the one still
  wanted.** Dragging the voxel slider queues a request per frame; the
  worker abandons every stale one at its next stage boundary. Cancellation
  needed no separate mechanism — to the worker, cancelled and superseded
  are the same fact: nobody is waiting.
- **Cancellation is honest about its granularity.** The core reports
  progress from inside a pass over the points but takes no answer back, so
  Esc is obeyed after the current stage, and the status strip says
  "stopped after the current stage" rather than implying otherwise.
- **`prepare_cloud` now borrows** (§7.2's crate, amended): the viewer holds
  its cloud behind a handle it cannot give up, and re-preparing on every
  slider movement would otherwise copy a million points each time.
- **Two threshold lines, not one.** The core classifies against the
  tolerance *and* against ten times it, so both are drawn. The
  classification is then something you read off the picture, and colour
  repeats position rather than carrying it alone.
- **A cancelled analysis has a way back**: "run again", and Space.

**M3 — Registration. Done.** Two clouds with roles, `register_observed`
streaming every accepted iteration as it happens, the source moving while
the solver is still working, the iteration strip, residual colouring, and
the wrong-basin warning.
*Gate: passed.* The test registers a generated corridor pair through the
viewer's own path and requires the command line's output to contain the
viewer's translation line, its RMSE-and-correspondences line and its whole
conditioning report, formatted exactly as the CLI prints them. Checked by
mutation: a 0.05 voxel against 0.07 fails it.

What the milestone showed, which is the point of having built it:

> `x −0.0300   y −0.0019   z −0.0100 m`, against a true offset of
> `−0.03, −0.02, −0.01`. Two axes exact to four decimals and the third
> wrong by 18 mm — and σ₆, the only bar past the tolerance line, is
> `ρ = [0, +1, 0]`: translation along the corridor. The README's opening
> example, on screen, without being told where to look.

Decisions worth recording:

- **A pose change is sixty-four bytes.** Each cloud carries a model matrix;
  the points never move. A registration that streams two hundred
  iterations writes two hundred uniforms and re-uploads nothing, which is
  what makes watching the solver affordable at two million points.
- **The model matrix folds in the origins.** Each cloud stores `f32`
  offsets from its own `f64` origin, and the two origins differ. The matrix
  is built as `pose · (origin_cloud + local) − origin_frame` in `f64` and
  narrowed once, so a georeferenced pair keeps its millimetres.
- **Residual colouring is drawn on the downsampled source**, because that
  is the cloud the solver used. Colouring the full-density source would
  cost a nearest-neighbour query per raw point to say the same thing, and
  would quietly imply the residual had been computed there. The panel says
  which cloud is on screen.
- **The wrong-basin warning is implemented**, not merely planned: a
  residual more than three times the stated sensor noise fades the spectrum
  and says why. Conditioning describes the shape of the cost function
  around wherever the solver stopped and has nothing to say about whether
  that was the right place.
- **Cancellation is now per iteration** where it matters. §7.1's
  `ControlFlow` earns its keep here: Esc during a solve is answered after
  the next accepted step rather than after the whole run.
- **The engine caches prepared surfaces** by cloud generation and
  parameters. Re-registering after a slider moves does not re-prepare what
  did not change.

A bug this milestone is worth naming, because the class of it will recur:
the composite shader kept the old `Frame` layout after a field was removed
from the cloud shader and from Rust, and wgpu rejected the first draw with
"bound with size 96 where the shader expects 112". Neither the compiler nor
the tests could see it — only running the thing could. Two shaders sharing
one uniform buffer must be edited together, and both files now say so.

**M4 — Null-space visualisation. Done.** Hover a σ row and the cloud
swings along that direction while every point takes the colour of its own
`|n·v|`; three demo scenes; and the command line that reproduces what is on
screen, copied to the clipboard.
*Gate: passed, and as a number rather than a recording.* The harness pins a
row and stops the clock at a chosen point in the swing, so two runs differ
only by the motion under test. Moving the corridor by the same 101 mm along
each direction changes

| direction | viewport pixels changed |
|---|---|
| σ₁ — across the corridor | **6.03 %** |
| σ₆ — along the corridor | **0.43 %** |

Fourteen times less response to an identical perturbation, which is the
whole claim, measured instead of asserted. A screen recording would have
shown the same thing and proved nothing.

The colouring says the same in a second way, and this is the part worth
looking at: hovering σ₁ lights up **the two walls and not the floor** —
the floor's normal is perpendicular to the motion, so it contributes
nothing to resisting it — while hovering σ₆ lights up almost nothing at
all. Correct physics, readable at a glance, from `n·(ρ + φ×p)` and nothing
else.

Two decisions inside it:

- **The amplitude is the same for every direction**, a fixed half percent
  of the scene, rather than each direction's own predicted spread. The
  experiment is *apply the same perturbation and watch the response*, and
  the response is exactly what σᵢ measures; scaling the motion by the
  spread would cancel the difference it exists to show.
- **`direction_in_world` is already a twist about the coordinate origin**,
  so the motion is `exp(a·ξᵢ)` and nothing needs centring by hand. The
  rotation lands about the centroid of the correspondences, where the
  report put it.

Not built, and deliberately: **the arrow field**. §5 lists three views of
one identity and this milestone ships two. Arrows would need a line
pipeline of their own to show the same `v` the swing already shows
integrated and the colour already shows projected. If a scene ever turns up
where neither reads, it can be added then.

**M5 — Polish. Done.** The command palette, a system-aware theme, a
failure banner that has room to be read, a drop overlay, empty states that
say what to do, and the README.
*Gate: passed on all three counts.*

- **No bare `String` anywhere.** There are no dialogs at all; a failure is
  a `PipelineError` shown in a banner across the top of the viewport, with
  the path intact, dismissed by clicking it. Twenty-six pixels of status
  strip used to truncate exactly the half that mattered.
- **Every command is reachable without the mouse.** ⌘K opens one list
  holding everything the application can be asked to do, filtered by
  typing, chosen with the arrow keys and return. While it is open the
  application's own shortcuts stand down — `f` is the letter f.
- **Both themes pass a contrast check**, and the check is a test rather
  than an opinion. `theme.rs` computes WCAG contrast ratios and asserts
  that every colour carrying text clears 4.5:1 against the panel, in both
  themes, and that the clouds clear 3:1 against the viewport and 1.4:1
  against each other. Five colours failed when it was first run and were
  re-derived by solving for the threshold rather than by eye — the light
  theme's σ colours were at 3.6:1, which is the *central number of the
  application* set in ten-pixel type below the readable limit.

Also settled here:

- **Every action goes through one path.** A button, a shortcut and a
  palette entry all call `run_command`, so they cannot drift apart. The two
  things a command cannot do without a `Context` — apply a theme, reach the
  clipboard — are deferred by a field and performed once in `ui`.
- **The window is frameless on macOS.** `fullsize_content_view` with the
  title bar transparent and the title hidden: the content runs to the top
  and the traffic lights float over it. egui's `with_titlebar_shown(false)`
  makes the bar transparent rather than removing it, and the buttons are
  governed by a separate setting left alone, so nothing can become
  unclosable. The inspector leaves 22 points of room for them, and the name
  at the top of the panel is the drag handle a missing title bar would
  otherwise cost.
- **The theme follows the system** at startup, and `RIGIDITY_UI_THEME`
  overrides it — which is also how the light theme got looked at without
  anyone pressing anything.

**M6 — Packaging. Done.** `build/rigidity.app` from a thirty-line script,
an icon generated by a script of its own, and a workflow for the three
platforms.
*Gate: passed, with one part of it stated rather than tested.* Launched the
way the Finder launches it (`open build/rigidity.app --args …`) the bundle
comes up with launchd as its parent — no terminal anywhere in the chain —
and loads both clouds. What was *not* exercised is the drop event itself:
dragging a file onto a window cannot be simulated from a shell. The path it
runs is the one ⌘O and the command line already use — all three end in
`engine.load` — so what is untested is winit delivering the event, not
anything of ours.

- **The icon is a script.** `package/icon.py` draws four bars on a dark
  tile, three short and teal and one long and amber crossing the line that
  marks the required accuracy — the application's own picture, and at
  sixteen pixels it still reads as *one of these is not like the others*.
  Written rather than drawn so it can be regenerated and argued with
  instead of being a binary nobody can account for. No dependencies: a PNG
  is a zlib stream, and axis-aligned rectangles need only a four-times
  supersample.
- **No document types in the plist.** Declaring `.ply` would put the
  application in "Open With" and on the Dock's drop target, where files
  arrive as an Apple event it does not answer. A promise that does nothing
  is worse than no promise.
- **The bundle is unsigned**, and the script says so where it is built
  rather than leaving it to fail mysteriously on someone else's machine.
- **CI cannot run yet, and says why.** The workflow needs the core beside
  it, and the core has no remote. The address is a repository variable
  rather than a name written into the file: a workflow naming a repository
  that does not exist looks configured and is not. The first step fails
  with the instruction if the variable is unset.

### Stage 2 — the workbench

A small, opinionated CloudCompare: the handful of its tools that get used
every day, and nothing else. The measure of success is not feature count —
it is that a person doing ordinary cloud work never has to leave for
something trivial and come back.

The rule that keeps this honest: **nothing here mutates a loaded cloud.**
Every operation produces a new entry in the scene list, the input stays
where it was, and undo is therefore free and total. That single decision
removes an undo stack, a dirty-state model and a save-before-quit dialog
from the application.

**W1 — Many clouds. Done.** The scene is a list. Each entry has a colour,
a visibility switch and two role chips; *source* and *target* are roles
given to two of the entries rather than a pair of loaders, so a third cloud
can be loaded for context and the roles reassigned without reloading
anything.
*Gate: passed on the clause that mattered.* Five clouds loaded at once cost
**five coordinate uploads** — the harness counts them and prints the total.
Hiding is instant by construction rather than by measurement: a hidden
cloud stays in the draw list and is skipped at the draw call, so the buffer
cache never sees it leave. Evicting on invisibility would have made the
tick a one-second pause, which is the opposite of what the tick is for.

The bug the gate was written to prevent was real and was there: buffers
were indexed by draw order, so hiding the second of three clouds handed the
third the second's coordinates and re-uploaded everything behind it. They
are keyed by the cloud now.

Two rules make the chips work, and they are a tested function rather than
a paragraph: a cloud cannot hold both roles, and clicking the role it
already holds puts it down — which is the only way back from a registration
to analysing one surface on its own.

Not built: a per-entry point size, and a per-entry transform beyond the
registration's own. Both are listed in this plan's original sentence and
neither has a tool that needs it yet — the transform arrives with manual
alignment (W4), and a per-cloud point size is worth having when there is a
scene where one cloud is ten times denser than another, which is W5's
problem.

**W2 — Scalar fields. Done.** One representation for every number the
application computes about a cloud — height, a column the file carried, the
residual, the distance to another cloud — and therefore one histogram, one
pair of clamps and one shader path instead of a special case per quantity.
*Gate: passed, with the first clause restated to be checkable.* "C2C
reproduces the known offset" is not quite a claim about a corridor: the
offset there is partly *along* the corridor, and a distance between
surfaces cannot see that component at all. The test instead lifts a plane
along its own normal, where every point's nearest neighbour is exactly the
lift away, and requires the median within five percent. It also requires
the median to be **at least** the lift — the nearest sample is a sample and
not a foot of the perpendicular, so it sits `√(lift² + spacing²)` away,
about two percent over at these densities. The second clause is a test that
clamps a field and asserts the values are the same `Arc`, with the same
contents and the same histogram.

The corridor is still where the feature justifies itself, though not as a
test: measured *before* a registration its histogram is unmistakably
**bimodal**, a hump at 0.01 and another at 0.03 — the floor displaced in z
and the walls in x, with the 0.02 along the corridor contributing nothing
because a surface cannot see a slide along itself. Measured after, one hump
at the noise floor. The picture states the whole problem the project is
about, from a number that has nothing to do with conditioning.

Three things this milestone found, none of them predicted:

- **A measurement cancelled the registration.** One counter decided which
  request was still wanted, so asking a second question abandoned the
  first. Requests now supersede within a *lane* — reports in one,
  measurements in the other — because they are different questions and
  answering one is no reason to stop answering the other.
- **The distance was measured at the wrong pose.** Computed where the
  clouds were loaded, then painted on a cloud drawn where the solver put
  it: the colours described somewhere the points no longer were. It is
  measured at the current pose now, and a registration landing mid-flight
  re-asks the question — including for a measurement still running, which
  was the case the first fix missed.
- **The tests were racing over a fixture.** Two of them wrote the same file
  in the temporary directory, and a third read it half-written and failed
  as a truncated PLY. Each has its own name now.

Not built: reading LAS, and with it intensity. The path is generic —
anything in `PointCloud::attributes` appears in the picker by name — so
intensity arrives with the format at W5 and needs nothing here.

**W3 — Selection and geometry. Done.** Shift and drag draws a lasso;
whatever it encloses can be kept or deleted into a new cloud. A
cross-section cuts a slab of the scene without removing anything. `M` arms
a measurement, and two clicks give the distance between two points, drawn
where it was taken. Subsampling makes a new cloud at the voxel size the
inspector already shows.
*Gate: passed on all three.*

- **The count is exact, and counted twice.** A test lassoes the upper-right
  quadrant of a grid with an identity projection and compares against a
  count taken without going near the projection at all — `assert_eq`, not a
  tolerance, because this is a question with a right answer.
- **The original is untouched**, and the test says something stronger: keep
  and delete *partition* it, `kept.len() + rest.len() == before`, and the
  kept points are the ones asked for in the order asked. That invariant is
  what stage two's missing undo stack rests on.
- **The cross-section is free.** 1 050 000 points, four seconds of
  orbiting: 8.34 ms median with the slab on, 8.34 ms with it off. It is a
  test in the vertex stage, so a clipped point costs less than a drawn one
  — the p95 is *better* with it on (8.69 ms against 16.74 ms), because
  fewer splats reach the rasteriser.

**One gesture, not two.** This plan asked for box *and* lasso selection.
There is one: the lasso. A box is a shape a lasso can trace, a lasso does
things a box cannot, and two gestures would need a mode to choose between
them. The gate's "box crop" is a lasso with four corners, which is exactly
what the test draws.

**A correction to M5.** Three things that milestone claimed — the failure
banner, the drop overlay and the two-line empty state — were not in the
code. A patch had failed to apply and nothing caught it: the tests do not
touch the viewport, and the screenshots taken to check M5 happened to show
neither a failure nor a drag. They are there now, added here. The lesson is
not about the patch; it is that "I wrote it" and "it is in the binary" are
different claims, and only the second one counts.

**W4 — Manual alignment. Done.** Click matching points on the two clouds,
three at least; the closed form gives the motion between them; the
registration starts there instead of at the identity.
*Gate: passed, both halves.* A corridor turned thirty degrees is out of
reach from the identity and the test **requires ICP to fail** from there —
a tool that rescues something never in danger has not been shown to do
anything. Three pairs, picked a couple of centimetres off on each side
because a person clicking a corner in two scans is imprecise, bring it to
within 0.02 of the truth. A third assertion requires the solver to have
*improved* on the clicks: if the picked pose were already the answer, the
test would be measuring nothing.

**The mathematics went upstream**, as §7.5 said it should. `rigidity-core::
lie::absolute_orientation` is Horn/Kabsch — centre both sets, decompose
their correlation, take the rotation — with six tests against analytical
oracles: a known motion returns to twelve decimals, three pairs suffice and
two do not, three points on a line return `None` rather than inventing the
rotation about it that nothing determined, and a coplanar set does not come
back mirrored. That last one is the case the determinant check exists for:
without it the answer fits the points exactly and is the wrong motion.

**What is tested and what is not.** The closed form, the registration from a
given pose, and the two together are covered. The click-to-pair flow is not,
because a click cannot be simulated from a shell — the same limit that let
three M5 features ship missing. It is written, it compiles, and it wants a
human to try it before it is believed.

**W5 — Formats. Done.** PLY, LAS, LAZ, E57, PCD and CSV in;
everything but CSV out; `read` and `write` choose by extension so neither
front end keeps a list to fall behind on. The command line got every reader
in the same commit, without asking.
*Gate: passed, and it bit.* The round-trip test runs over every writable
format twice — once at the origin and once at a UTM coordinate half a
million metres east and four million north — and requires a millimetre.
**PCD failed the second one at 0.125 m**, because the writer put `f32`
absolute coordinates in the file, where the step at four million metres is
a quarter of a metre. That is precisely the mistake the core's storage
exists to prevent, made at the last possible moment. PCD now writes `f64`
against PCL's own convention, and says why in the module.

- **E57 could finally be written**, which is what let it be read with any
  confidence: `rigidity`'s own plan had deferred the format "for want of
  test data", and a format that can be written supplies its own. Reading
  concatenates the scans a file holds, each through its own transform,
  because everything upstream of the viewer works on one cloud at a time —
  stage three is where scans stay apart.
- **PCD is ours**, like PLY: a text header and a block of numbers, and a
  dependency for that costs more than the code does.
- **One bug worth naming.** The viewer's own loader still called `read_ply`
  directly after the pipeline and the CLI had learned the dispatcher. The
  only symptom was a failure banner on a file the command line opens
  fine — and, because a viewer with nothing loaded has nothing to animate,
  a window that sat there. The tests could not see it: they exercise the
  library, and this was a caller.

That closes stage two. The scene is a list, the numbers have a histogram, a
lasso and a slab and a ruler work on what is on screen, three clicks escape
a wrong basin, and the formats a survey actually arrives in go in and out.

### Stage 3 — the survey

Forty scans instead of two. This is the stage that justifies the whole
project, and it rests on one observation.

**Why a pose graph belongs in *this* application.** Every pairwise
registration already produces `IcpResult.information` — the matrix `JᵀWJ` at
the solution — which is exactly the information matrix a pose-graph edge
needs. Standard packages take it at face value. The README's own
measurement says that is wrong by a factor of about seventeen on real data,
and the conditioning analysis says something stronger and more useful: *it
says which directions of that matrix are worth anything at all.* An edge
built from a corridor should carry no weight along the corridor — not a
small weight, not a fabricated one, none — and every other tool in this
space either does not know that or cannot express it.

**Degeneracy-weighted pose-graph optimisation is the contribution.** The
viewer is how you see it; the mathematics belongs upstream in `rigidity`
(§7), with its own tests and the same determinism guarantee as everything
else there.

**S1 — Projects.** Many scans, each with a pose and its own parameters;
saved and reopened. The project file references clouds by path and stores
poses as text; it is the viewer's format, not the core's.
*Gate:* a project of twenty scans reopens with every pose bit-identical —
poses are written at full `f64` precision, because a project that drifts on
save is a project that cannot be trusted to measure drift.

**Gate passed.** `src/project.rs`, and `Entry` gained a pose. What decided
the encoding was the word *bit-identical*: twelve numbers per pose, the
rotation matrix and the translation, not the six of a twist. `Se3` stores a
matrix, so writing `log` and reading `exp` would round-trip through two
transcendental functions and come back near the pose rather than at it.
Twenty scans at UTM coordinates with rotations from `exp` — so the entries
are the irrational-looking `f64`s a solver actually produces — come back
with the same bits, through a string and again through a file. Rust's own
float formatting is the shortest decimal that parses back to the identical
value, so no precision is specified anywhere; `{:.17}` would be longer, no
more exact, and would suggest a decision had been made.

Two invariants are tested rather than assumed, because they fail silently.
Opening a project must not move a scan: `initial` is set to the motion the
stored poses already describe, so the source is drawn at exactly the pose
the file gave it and the solver starts from the survey rather than from the
identity. And placing a registration must not move anything either — it
changes what a position is attributed to, not the position.

**Parameters per scan** came with it. `PrepareParams` moved from the
application to the `Entry`, and `register` takes one for each side rather
than one for the pair: a survey is not made of clouds at one density, and
making the pair agree lets the coarser of the two decide for both. The
sliders name the scan they are about and leave their values as the seed for
the next scan loaded, so the common case — twenty scans that agree — still
costs one adjustment.

What that test asserts is the point count each side kept, not the pose. A
run where the second set of parameters was dropped on the way and the first
used for both would converge and look entirely right; the kept count is the
only number that can come from nothing but that side's own voxel.

Two smaller consequences, both recorded rather than smoothed over. Only a
scan tuned away from the default writes `voxel` and `neighbours` into the
project, because a file that spells out values nobody chose reads as though
somebody did — and a reader that helpfully filled in the defaults would make
every project ever saved immune to the next release changing one. And the
copied command line carries one `--voxel` for a pair: when the two scans
disagree it reproduces the target's settings and says so in a comment,
rather than printing something that runs and gives different numbers.

### The level-of-detail decision — made against measurements

Twenty scans of a million points is twenty times the M1 budget, and the
plan deferred the choice between octree LOD, out-of-core and downsampled
copies until there was a number. There is now. Apple M5, Metal, a 2560×1600
viewport, the camera orbiting throughout:

| on screen | median frame | |
|---|---|---|
| 0.19 M — one real scan | 8.33 ms | 120 fps, vsync |
| 2 M — two scans | 16.67 ms | 60 fps, vsync |
| 2.7 M — **twenty** scans | 16.73 ms | 60 fps |
| 2.7 M — **three** scans | 25.00 ms | 40 fps |
| 3.8 M — twenty real ETH scans | 36.71 ms | 27 fps |
| 5 M — five scans | 33.33 ms | 30 fps |
| 10 M — ten scans | 62.51 ms | 16 fps |
| 20 M — twenty scans | 120.83 ms | 8 fps |

The cost is linear in points at about 6.2 ms per million, which puts the
budget at roughly **2.7 M points on screen** for 60 fps. Twenty scans of a
million each is seven times over, and the guess in the paragraph above was
right.

The third and fourth rows are the ones that decided it. The same 2.7 M
points draw *faster* spread across twenty clouds than packed into three,
because three stations occupy a third of the screen and overlap: this
renderer is fill-bound, and twenty draw calls cost nothing worth naming.
An octree's benefit is fewer, larger draws — a benefit against a cost that
is not there. Out-of-core is ruled out on memory rather than on principle:
20 M points is 240 MB of coordinates across three `f32` buffers, which a
unified-memory machine does not notice.

**So: downsampled copies, drawn while the camera moves.** Each scan over
135 000 points gets a voxel-downsampled preview when it loads, and the
viewport draws previews whenever the scene is over budget *and* the camera
moved within the last 200 ms. A still survey costs its one slow frame and
then nothing, because egui does not repaint what nobody is touching. The
renderer already keys coordinates by an opaque `u64` and never evicts, so
both copies stay resident and swapping between them costs no upload — which
is why this is a few lines rather than a spatial index.

Measured after: 20 M points orbits at **120 fps**, up from 8. The twenty
real ETH scans go from 27 fps to 120. A cloud carrying a scalar field is
never swapped for its preview, because the values are one per point of the
full cloud.

135 000 is a cap and not a target, and the difference matters when reading
the number above. The voxel is guessed from the bounding box and then
corrected only downwards, so a preview usually lands well under: the
corridor demo keeps 50 271 points of 180 000, 28%. That is why twenty
million came back at 120 fps rather than at the 60 the cap alone predicts,
and it is why the unblock condition below is a measurement rather than a
scan count.

*Unblocks an octree:* a survey whose **previews alone** exceed the budget,
where lowering the cap further stops showing the scene. The cap is a knob
long before it is a rewrite, and the guess currently leaves most of the
knob unturned — so this is further off than forty scans, but it is on the
way there rather than hypothetical.
*Unblocks out-of-core:* full clouds that no longer fit in memory.

**S2 — Pose graph.** Gauss–Newton on SE(3) over the graph, with each edge's
information matrix filtered through its own conditioning: directions whose
predicted spread exceeds the tolerance contribute nothing. Dense normal
equations to a few hundred poses (a 1200×1200 solve is milliseconds);
sparse only when a real survey demands it.
*Gate:* a synthetic loop with known poses and one deliberately degenerate
leg — the weighted optimisation ends with measurably less drift than the
naive one, and both are bit-for-bit reproducible at any thread count.

**S3 — Loop closure.** Manual first: pick two scans, register them, add the
edge, re-optimise. Detection afterwards, and only if the manual path proves
it is worth it.
*Gate:* manual closure removes the drift from a synthetic loop; any later
detector is measured as precision and recall against the same loop, not
demonstrated on a screenshot.

**S4 — Whole-survey view.** The σ panel at survey scale: residual per edge,
conditioning per scan, drift along the trajectory.
*Gate:* the deliberately weak leg of a synthetic survey is identifiable
from this view alone, by someone who was not told where it is.

---

## 9. Anti-scope

Each entry has the condition under which it may be reopened. Without a
condition it is not an anti-scope entry, it is a mood.

Four entries were removed when stages 2 and 3 were adopted: cropping and
measurement (now W3), manual alignment (now W4, and a better tool than the
gizmo this plan first imagined), projects and the registration graph (now
S1 and S2), and E57 (now W5). They are recorded here as removed rather than
deleted, because an anti-scope list that quietly loses its entries teaches
nobody anything. Level-of-detail moved too: it is no longer refused, it is
scheduled — the decision happens at S1's gate, against a real point count.

- **Not a CloudCompare clone.** Stage 2 is ten tools, not two hundred. Every
  candidate for an eleventh has to displace one of the ten. *Unblock:* never
  as a blanket; each tool argues for itself against that bar.
- **No meshing, no surface reconstruction, no texturing.** Poisson
  reconstruction and its relatives are a different problem with a different
  literature, and MeshLab is right there. *Unblock:* never.
- **No rasters, no DEMs, no GIS layers.** *Unblock:* never; that is QGIS.
- **No live or online SLAM.** Stage 3 is post-processing: scans on disk,
  poses solved at leisure, every result reproducible. A live front end has
  hard real-time constraints and an entirely different failure model.
  *Unblock:* never in this application.
- **No photogrammetry, no image input.** *Unblock:* never.
- **No web build.** WebGPU plus a million points plus local file access is a
  different product with a different plan. *Unblock:* never.
- **No plugins, no scripting, no embedded Python.** *Unblock:* never; that is
  what the CLI is for, and `rigidity-pipeline` is its library.
- **No destructive editing.** Not a limitation but a design decision, and the
  one that pays for stage 2's simplicity: every operation makes a new cloud,
  the input is never touched, and so there is no undo stack, no dirty state
  and no save-before-quit dialog anywhere in the application. *Unblock:*
  only if memory pressure makes copies untenable, which is an S1 question.

---

## 10. Risks

- **Version triple.** ~~Resolved.~~ Depending on `eframe` alone and using
  its re-exports removes the possibility of a mismatch: `eframe::egui`,
  `eframe::egui_wgpu` and `eframe::wgpu` are by construction the ones it was
  built against. Resolved: 0.36.1 / 0.36.1 / 30.0.0.
- **The toolchain pin holds.** ~~Resolved.~~ The whole stack builds on
  1.97.1, so the viewer and the CLI are compiled by the same rustc and their
  numbers stay comparable.
- **Path dependencies make an uncommitted repository a build input.**
  ~~Resolved.~~ `../rigidity` is committed, has a remote, and is on
  crates.io as 0.1.1 — and moving to a git or version dependency has
  stopped being the goal rather than become possible. Stage 3 writes
  `rigidity-graph` upstream (§7.7) and calls it from here in the same
  sitting; a pinned dependency would put a publish between every two edits.
  CI resolves it the other way, checking both repositories out side by side
  by name. Revisit when a builder who is neither this machine nor CI has to
  resolve these paths — the README's build section is the interim answer,
  and it says clone both.
- **wgpu's API moves between majors.** wgpu 30 renamed
  `push_constant_ranges` to `immediate_size` and made both
  `bind_group_layouts` and `VertexState::buffers` take `Option`s; egui 0.36
  replaced `SidePanel`/`TopBottomPanel` with one `Panel` and moved
  `eframe::App` from `update(&Context, …)` to `ui(&mut Ui, …)`. None of that
  is in any tutorial yet. Read the vendored source in
  `~/.cargo/registry/src/*/` before writing against a remembered API; it is
  faster than a compile-fix loop and much faster than a plausible-looking
  wrong answer.
- **Two target directories.** A separate workspace rebuilds the core rather
  than sharing artefacts. That is the price of an independent lock file — and
  the independent lock file is what keeps a wgpu/egui stack from having to
  agree with whatever `rerun` pins. Accepted deliberately.
- **A space in the repository path.** The parent directory is
  `point cloud head`. Cargo handles it; shell scripts and CI snippets that
  forget to quote do not. Quote every path in every script from the start.
