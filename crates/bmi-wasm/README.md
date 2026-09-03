# bmi-wasm

The bmi-driver BMI adapters compiled to WebAssembly, so BMI models can be loaded and driven
from JavaScript (browser or Node) **without modifying the model source**.

## How it works

`wasm-pack`/`wasm-bindgen` can't do this: their `wasm32-unknown-unknown` target has no
`dlopen`, no libc, and no filesystem, so a model can only be used by statically linking it
into the same wasm binary. Instead this crate targets **`wasm32-unknown-emscripten`** and
uses Emscripten's dynamic linking:

- This crate depends on [`bmi-core`](../bmi-core) (default features off, so no Fortran/Python
  adapter) and builds it — `BmiC`, the `library.rs` dlopen code — into a **main module**
  (`pkg/bmi.mjs` + `pkg/bmi.wasm`). `MAIN_MODULE=1` keeps the full libc in it so models can
  resolve any symbol against it.
- Each model is compiled unchanged with `emcc -O2 -sSIDE_MODULE=2 -o model.wasm model.c`
  — the wasm equivalent of building a `.so`. The BMI registration function must be exported
  (`EMSCRIPTEN_KEEPALIVE` on it, or `-sEXPORTED_FUNCTIONS=_register_bmi_...`).
- At runtime the side module is written into Emscripten's in-memory filesystem (MEMFS) and
  the adapter literally calls `dlopen`/`dlsym` on it — the same code path as native. Config
  files the model `fopen`s are staged into MEMFS the same way.

Layers:

- `src/api.rs` — C ABI over the adapters: `i32` model handles, `bmi_last_error()`,
  Rust-allocated buffers with explicit free functions.
- `js/bmi.mjs` — the API you actually use: `BmiRuntime` / `BmiModel` classes that handle all
  pointer allocation, string conversion, and typed-array copies.

## Build

```bash
rustup target add wasm32-unknown-emscripten
# install emsdk: https://emscripten.org/docs/getting_started/downloads.html
./build.sh          # -> pkg/bmi.mjs + pkg/bmi.wasm
./build.sh --test   # also compiles test/model.c as a side module and runs test/run.mjs
```

## Usage

```js
import createBmiModule from './pkg/bmi.mjs';
import { BmiRuntime } from './js/bmi.mjs';

const rt = await BmiRuntime.load(createBmiModule);

// Stage the compiled model and its config into the in-memory FS.
rt.writeFile('/models/cfe.wasm', cfeBytes);          // emcc -sSIDE_MODULE=2 build
rt.writeFile('/config/cfe.ini', configText);

const model = rt.openModel('cfe', '/models/cfe.wasm', 'register_bmi_cfe');
model.initialize('/config/cfe.ini');

console.log(model.getComponentName());
console.log(model.getInputVarNames(), model.getOutputVarNames());
console.log(model.getVarInfo('land_surface_water__runoff_volume_flux'));

for (let t = 0; t < 24; t++) {
  model.setValue('atmosphere_water__precipitation_leq-volume_flux', [1e-6]); // number[] or typed array
  model.update();
  const q = model.getValue('land_surface_water__runoff_volume_flux');        // Float64Array
}

model.finalize();
model.close();
```

`getValue` returns a `Float64Array`, `Float32Array`, or `Int32Array` matching the model's
declared type; `setValue` converts whatever you pass to that type. Several models can be
open at once (each `openModel` gets its own handle, and each registration gets fresh
`bmi->data` state). Errors from any BMI call throw a JS `Error` carrying the adapter's
error message.

## Example page

`example/index.html` is a drag-and-drop sandbox: drop a compiled model side module and a
config file (either complete, or a Python-format-style template like `gw_depth = {depth}` —
placeholders become sliders in their own column, and changing one restarts the model since
BMI initializes once). Placeholders can embed a default and slider bounds:
`{smcmax=0.439}` or `{smcmax=0.439,0,1}` (optionally with a format spec, e.g.
`{leak=0.1,0,1:.3f}`); bare `{name}` defaults to 1 with a 0–10 range. The registration
function is auto-detected from the side module's exports. After initialize it builds one
slider per model input variable and one rolling chart per output variable, plus a table of
recent steps.

Bundled examples: files in `example/models/` appear in dropdowns so nobody has to drag
their own — `.wasm`/`.so` files as models, `.ini`/`.input`/`.txt`/`.cfg`/`.template` as
configs. The page reads `example/models/manifest.json` (a JSON array of filenames — keep it
updated when adding files), falling back to the server's directory listing (works with
`python3 -m http.server`) when there is no manifest.

```bash
./build.sh --test                      # builds pkg/ and test/model.wasm
python3 -m http.server -d . 8000      # serve the wasm/ directory
# open http://localhost:8000/example/ and drop test/model.wasm + test/bucket.cfg.template
```

## Recompiling real models (e.g. CFE)

A native `.so` (x86-64 ELF from gcc) can never run in wasm — but the same unmodified
source recompiles into a loadable side module. Verified with NOAA-OWP CFE:

```bash
git clone https://github.com/NOAA-OWP/cfe.git && cd cfe
emcc -O2 -sSIDE_MODULE=2 -sEXPORTED_FUNCTIONS=_register_bmi_cfe \
  -Iinclude src/bmi_cfe.c src/cfe.c src/giuh.c src/conceptual_reservoir.c \
  src/nash_cascade.c -o cfe.wasm
```

Drop `cfe.wasm` plus a config with `forcing_file=BMI` (e.g. `configs/cfe_config_cat_87_pass.txt`,
a copy is in `test/cfe_config.txt`) into the example page, or dlopen it from JS. The only
emcc-specific piece is `-sEXPORTED_FUNCTIONS=_<registration fn>` — no source changes.
The same recipe applies to other C models (PET etc.); C++ models build with `em++`;
Fortran models are the exception (no Fortran compiler in emsdk — see below).

## Notes & limitations

- Only the C adapter (`BmiC`) is exposed. `BmiPython` can't build for wasm (pyo3), and
  `BmiFortran` would need a Fortran-to-wasm compiler (LLVM flang can emit wasm these days,
  but emsdk doesn't ship it) — compile Fortran models behind a C shim for now.
- `pkg/bmi.wasm` is ~1.5 MB because `MAIN_MODULE=1` bundles all of libc. If size matters,
  switch to `-sMAIN_MODULE=2` in `.cargo/config.toml` and list the libc symbols your models
  need, or link the side modules at build time with `RUNTIME_LINKED_LIBS`.
- Models with their own heavy dependencies (NetCDF, etc.) need those libraries also compiled
  to wasm and dlopen'd first (`RTLD_GLOBAL` is set, so their symbols become visible), or
  statically linked into the side module.
- The main module and side modules must be built with compatible Emscripten versions
  (the dynamic-linking ABI is not guaranteed stable across major releases).
- `-sGROWABLE_ARRAYBUFFERS=0` is required with Emscripten 6.0.2: its default auto-detects
  resizable ArrayBuffers (supported by current Chrome), but the glue's `UTF8ToString` then
  crashes inside `dlsym` because `TextDecoder.decode()` rejects resizable buffers. Works in
  Node either way (its V8 doesn't use resizable wasm memory), so test in a real browser.
  Revisit when upstream fixes the decode path.
