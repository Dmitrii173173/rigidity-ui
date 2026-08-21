# rigidity-ui — development plan

Plan version: 2026-08-21. Crate versions are the ones `Cargo.lock`
actually resolved: eframe/egui/egui-wgpu 0.36.1, wgpu 30.0.0, nalgebra
0.35.0. Everything is vendored in the local registry, and
`cargo build --offline` is green.

Companion documents: [`../rigidity/README.md`](../rigidity/README.md),
[`../rigidity/PLAN.md`](../rigidity/PLAN.md),
[`../rigidity/MOTIVATION.md`](../rigidity/MOTIVATION.md).

## Status

| Milestone | State | Note |
|---|---|---|
| M0 — Scaffold | **done** | window, wgpu callback, shell layout, theme |
| M1 — Cloud rendering | not started | splats, orbit camera, EDL |
| M2 — Pipeline and spectrum | not started | worker thread, `analyse`, σ panel |
| M3 — Registration | not started | live ICP, iteration timeline |
| M4 — Null-space visualisation | not started | the feature the app exists for |
| M5 — Polish | not started | theme, palette, keyboard, errors |
| M6 — Packaging | not started | `.app` bundle, CI on three OS |
| §7 — upstream changes | **done** | all four landed in `../rigidity`; CLI output unchanged |

---

## 0. What this is, in one paragraph

`rigidity` answers a question no other registration tool answers: *which
degrees of freedom did the geometry actually determine?* The CLI prints that
answer as six lines of σ values. Six lines is the right output for a script
and the wrong output for a human — nobody looks at `ρ=[+0.00 +1.00 +0.00]`
and pictures a corridor sliding along itself. **This application exists to
make the ambiguous direction visible.** Everything else it does — loading,
rendering, registering — is infrastructure in service of that one screen.

Anything the CLI already does well stays in the CLI. If a feature does not
end in something you can *see*, it does not belong here.

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
# Path dependencies — rigidity is unpublished, so a path dependency is the
# only option today. Switch to a git dependency once the repository has a
# remote; §10.
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

Note for M1: `nalgebra`'s bytemuck feature is spelled `convert-bytemuck`,
and a `Matrix4<f32>` behind it goes into a uniform buffer with no
conversion layer. That is what keeps a second linear-algebra crate out.

**Why this and not something else.**

| Alternative | Why not |
|---|---|
| Tauri / Electron + three.js | The data path is the product. Getting a million points into JS means serialising them — and `rigidity-core` has no `serde` at all. Adds a second toolchain to a project whose selling point is not needing one. |
| Bevy | An ECS and an asset pipeline for an application with two clouds and one panel. Its UI layer would end up being `bevy_egui` anyway. |
| iced | The most attractive default look of the Rust options, and `iced::widget::shader` can host a wgpu pass. But the 3D viewport *is* the application here, not an inset; `egui-wgpu`'s paint callback is the shorter, better-trodden path to it. Revisit if the panel work ever outgrows immediate mode. |
| Slint | Same viewport objection, plus a licence conversation the core's MIT/Apache does not need. |
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
    engine/               # M2 — everything that touches rigidity
      mod.rs              #   worker thread, channels
      job.rs              #   Job / Event enums
      session.rs          #   the pipeline call sequence and its state
      error.rs            #   typed errors, no String
    render/
      mod.rs              # ✓ pipeline, callback, resources
      viewport.wgsl       # ✓ M0 triangle; becomes the splat shader at M1
      camera.rs           # M1 orbit / pan / dolly, fit-to-bounds
      cloud.rs            # M1 vertex-pulled splats
      edl.rs              # M1 eye-dome lighting post pass
    panels/               # M2 onwards; M0 keeps them inline in app.rs
      inspector.rs        # sources, parameters
      spectrum.rs         # the six σ rows — the centrepiece
      status.rs           # progress strip
      palette.rs          # ⌘K
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

**The one interaction that matters.** Hovering σ₆ does not highlight a row.
It takes the null-space direction from `Conditioning::direction_in_world(5)`
and gently oscillates the source cloud along it, with an amplitude of a few
times the predicted spread. On the corridor scene the cloud slides along the
corridor and *nothing appears to change* — which is precisely what "this
degree of freedom is not determined" means, shown rather than asserted.
Hovering σ₁ does the same and the walls visibly tear apart. Release, and it
eases back to the registered pose in 160 ms.

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
  callback; scrubbing is a uniform write.
- **RMSE is never far from the spectrum.** The README is explicit that
  conditioning cannot detect a wrong local minimum, and that on real data 11
  of 30 pairs converged to a wrong basin with perfectly healthy spectra. A UI
  that shows a confident spectrum without the residual next to it would be
  actively misleading. When RMSE is high relative to the noise floor, the
  spectrum panel is marked as unreliable rather than merely accompanied by a
  number.

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
- **One interleave on upload.** `columns()` returns three slices; the
  renderer builds `Vec<[f32; 3]>` once per cloud generation. That copy is
  unavoidable and is the only one.
- **Reverse-Z `f32` depth**, so a scene spanning four orders of magnitude
  does not z-fight at the far end.
- **Eye-dome lighting** as a full-screen post pass over depth. Point clouds
  without normals are unreadable without it; with it, they read as surfaces.
  This is the single cheapest thing that separates "looks modern" from
  "looks like 2009".
- **Budget.** One million points is four million vertices — comfortable at
  60 fps on Apple Silicon. Above roughly five million, render the
  voxel-downsampled cloud during interaction and the full one when the
  camera is idle. Octree LOD and out-of-core streaming are out of scope (§9).

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

---

## 8. Milestones

Each milestone has a gate. A gate is a thing that either passes or does not;
"looks fine" is not a gate.

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

**M1 — Cloud rendering.** Load a PLY through `rigidity-io`, upload, orbit
camera, fit-to-bounds from `PointCloud::bounds()`, splats, EDL, point-size
control.
*Gate:* `demo/corridor_target.ply` at one million points holds 60 fps while
orbiting, measured with a frame-time readout, on the development machine.

**M2 — Pipeline and spectrum.** Worker thread, jobs, progress, cancel
between stages. Single-cloud `analyse`. The σ panel with live noise and
tolerance sliders.
*Gate:* a test runs the UI's own pipeline and `rigidity analyse` on
`demo/corridor_target.ply` and asserts the six singular values agree
exactly, not approximately.

**M3 — Registration.** `register_observed` streaming into the timeline, live
pose during the run, residual colouring, RMSE in the status strip.
*Gate:* pose, RMSE and correspondence count match
`rigidity register demo/corridor_source.ply demo/corridor_target.ply`
exactly, asserted in a test.

**M4 — Null-space visualisation.** Hover-to-oscillate, contribution
colouring, "copy CLI command", demo scenes from `rigidity-scenes`.
*Gate:* on the corridor pair, hovering σ₆ moves the cloud visibly along the
corridor axis and the rendered image barely changes; hovering σ₁ tears the
walls apart. Recorded as a short screen capture attached to the milestone.

**M5 — Polish.** Theme, palette, drag and drop, empty states, typed error
surfaces, full keyboard coverage, README.
*Gate:* no dialog anywhere displays a bare `String`; every command in the
palette is reachable without the mouse; both themes pass a contrast check.

**M6 — Packaging.** macOS `.app` bundle with an icon and an Info.plist, CI
building on Linux, macOS and Windows against the pinned toolchain.
*Gate:* a double-clickable application that opens a file by drag and drop
with no terminal involved.

---

## 9. Anti-scope

Each entry has the condition under which it may be reopened. Without a
condition it is not an anti-scope entry, it is a mood.

- **No cloud editing, cropping, annotation or measurement.** *Unblock:*
  never. That is CloudCompare's job and it does it well.
- **No project or session management, no registration graph, no SLAM.**
  *Unblock:* when someone needs to register more than two clouds twice in a
  week.
- **No octree LOD or out-of-core streaming.** *Unblock:* when a real dataset
  above twenty million points has to be shown at full density.
- **No E57.** *Unblock:* when `rigidity-io` learns to read it — the format is
  declared in the workspace dependencies but no crate uses it.
- **No web build.** *Unblock:* never. WebGPU plus a million points plus local
  file access is a different product with a different plan.
- **No plugins, no scripting, no embedded Python.** *Unblock:* never; that is
  what the CLI is.
- **No manual alignment gizmo.** *Unblock:* plausible, and the likeliest M7.
  The README is clear that a wrong initial pose sends ICP into a wrong basin
  that no spectrum will flag; a coarse manual pre-alignment is the honest
  answer to that, and a 3D gizmo is the only way to offer it. Deferred, not
  rejected.

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
  ~~Resolved.~~ `../rigidity` now has two commits: the state as found, and
  the §7 changes on top. Moving to a git dependency is still the goal and
  needs a remote.
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
