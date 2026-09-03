// JavaScript convenience wrapper around the bmi-wasm Emscripten module.
//
// Usage:
//   import createBmiModule from '../pkg/bmi.mjs';
//   import { BmiRuntime } from './bmi.mjs';
//
//   const rt = await BmiRuntime.load(createBmiModule);
//   rt.writeFile('/models/cfe.wasm', sideModuleBytes);   // model compiled with emcc -sSIDE_MODULE=2
//   rt.writeFile('/config/cfe.ini', configText);
//   const model = rt.openModel('cfe', '/models/cfe.wasm', 'register_bmi_cfe');
//   model.initialize('/config/cfe.ini');
//   model.setValue('atmosphere_water__precipitation_rate', [1e-6]);
//   model.update();
//   const q = model.getValue('land_surface_water__runoff_volume_flux'); // Float64Array
//
// All pointer allocation/free for strings, out-params, and value buffers is handled here.

const TYPE_F64 = 0;
const TYPE_F32 = 1;
const TYPE_I32 = 2;

export class BmiRuntime {
  constructor(Module) {
    this.M = Module;
  }

  /** Instantiate the Emscripten module factory and wrap it. */
  static async load(createModule, moduleOptions = {}) {
    return new BmiRuntime(await createModule(moduleOptions));
  }

  get FS() {
    return this.M.FS;
  }

  /** Write a file into the in-memory filesystem, creating parent directories. */
  writeFile(path, data) {
    const dir = path.slice(0, path.lastIndexOf('/'));
    if (dir) this.mkdirp(dir);
    this.M.FS.writeFile(path, typeof data === 'string' ? data : new Uint8Array(data));
  }

  mkdirp(dir) {
    const parts = dir.split('/').filter(Boolean);
    let cur = '';
    for (const part of parts) {
      cur += '/' + part;
      try {
        this.M.FS.mkdir(cur);
      } catch (e) {
        if (e.errno !== 20 /* EEXIST */) throw e;
      }
    }
  }

  readFile(path, encoding = 'utf8') {
    return this.M.FS.readFile(path, { encoding });
  }

  lastError() {
    const ptr = this.M._bmi_last_error();
    return ptr ? this.M.UTF8ToString(ptr) : 'unknown error';
  }

  /** Run `fn` with C-string copies of `strs`, freeing them afterwards. */
  #withCStrings(strs, fn) {
    const ptrs = strs.map((s) => this.M.stringToNewUTF8(s));
    try {
      return fn(...ptrs);
    } finally {
      ptrs.forEach((p) => this.M._free(p));
    }
  }

  /**
   * dlopen a model side module already present in the filesystem and register its BMI
   * struct via `regFunc`. Returns a BmiModel.
   */
  openModel(name, libPath, regFunc = 'register_bmi') {
    const handle = this.#withCStrings([name, libPath, regFunc], (n, l, r) =>
      this.M._bmi_open(n, l, r)
    );
    if (handle < 0) throw new Error(`bmi_open failed: ${this.lastError()}`);
    return new BmiModel(this, handle, name);
  }
}

export class BmiModel {
  constructor(runtime, handle, name) {
    this.rt = runtime;
    this.M = runtime.M;
    this.handle = handle;
    this.name = name;
  }

  #check(status, what) {
    if (status !== 0) throw new Error(`${what} failed for '${this.name}': ${this.rt.lastError()}`);
  }

  #takeString(ptr, what) {
    if (!ptr) throw new Error(`${what} failed for '${this.name}': ${this.rt.lastError()}`);
    const s = this.M.UTF8ToString(ptr);
    this.M._bmi_free_string(ptr);
    return s;
  }

  #withCString(str, fn) {
    const p = this.M.stringToNewUTF8(str);
    try {
      return fn(p);
    } finally {
      this.M._free(p);
    }
  }

  initialize(configPath) {
    this.#check(
      this.#withCString(configPath, (p) => this.M._bmi_initialize(this.handle, p)),
      'initialize'
    );
  }

  update() {
    this.#check(this.M._bmi_update(this.handle), 'update');
  }

  updateUntil(time) {
    this.#check(this.M._bmi_update_until(this.handle, time), 'update_until');
  }

  finalize() {
    this.#check(this.M._bmi_finalize(this.handle), 'finalize');
  }

  /** Finalize (if needed) and release the model handle. */
  close() {
    this.#check(this.M._bmi_close(this.handle), 'close');
    this.handle = -1;
  }

  getComponentName() {
    return this.#takeString(this.M._bmi_get_component_name(this.handle), 'get_component_name');
  }

  getInputVarNames() {
    return JSON.parse(
      this.#takeString(this.M._bmi_get_input_var_names_json(this.handle), 'get_input_var_names')
    );
  }

  getOutputVarNames() {
    return JSON.parse(
      this.#takeString(this.M._bmi_get_output_var_names_json(this.handle), 'get_output_var_names')
    );
  }

  /** {name, type, units, itemsize, nbytes, location, grid} — unimplemented fields are null. */
  getVarInfo(name) {
    return JSON.parse(
      this.#withCString(name, (p) =>
        this.#takeString(this.M._bmi_get_var_info_json(this.handle, p), 'get_var_info')
      )
    );
  }

  /** Returns a Float64Array, Float32Array, or Int32Array copy of the variable. */
  getValue(name) {
    return this.#withCString(name, (namePtr) => {
      const out = this.M._malloc(8); // two i32 out-params: len, type
      try {
        const bufPtr = this.M._bmi_get_value(this.handle, namePtr, out, out + 4);
        if (!bufPtr) {
          throw new Error(`get_value failed for '${name}': ${this.rt.lastError()}`);
        }
        // Re-read heap views after the call: memory growth invalidates cached views.
        const len = this.M.HEAP32[out >> 2];
        const type = this.M.HEAP32[(out + 4) >> 2];
        let result;
        switch (type) {
          case TYPE_F64:
            result = new Float64Array(this.M.HEAPF64.buffer, bufPtr, len).slice();
            break;
          case TYPE_F32:
            result = new Float32Array(this.M.HEAPF32.buffer, bufPtr, len).slice();
            break;
          case TYPE_I32:
            result = new Int32Array(this.M.HEAP32.buffer, bufPtr, len).slice();
            break;
          default:
            throw new Error(`get_value returned unknown type code ${type} for '${name}'`);
        }
        this.M._bmi_free_buffer(bufPtr, len, type);
        return result;
      } finally {
        this.M._free(out);
      }
    });
  }

  /** Accepts a typed array or plain number array; converts to the model's declared type. */
  setValue(name, values) {
    return this.#withCString(name, (namePtr) => {
      const type = this.M._bmi_get_var_type_code(this.handle, namePtr);
      let typed;
      switch (type) {
        case TYPE_F64:
          typed = values instanceof Float64Array ? values : Float64Array.from(values);
          break;
        case TYPE_F32:
          typed = values instanceof Float32Array ? values : Float32Array.from(values);
          break;
        case TYPE_I32:
          typed = values instanceof Int32Array ? values : Int32Array.from(values);
          break;
        default:
          throw new Error(`unknown variable type for '${name}': ${this.rt.lastError()}`);
      }
      const ptr = this.M._malloc(typed.byteLength);
      try {
        new Uint8Array(this.M.HEAPU8.buffer, ptr, typed.byteLength).set(
          new Uint8Array(typed.buffer, typed.byteOffset, typed.byteLength)
        );
        this.#check(
          this.M._bmi_set_value(this.handle, namePtr, ptr, typed.length),
          `set_value('${name}')`
        );
      } finally {
        this.M._free(ptr);
      }
    });
  }

  getCurrentTime() {
    return this.M._bmi_get_current_time(this.handle);
  }

  getStartTime() {
    return this.M._bmi_get_start_time(this.handle);
  }

  getEndTime() {
    return this.M._bmi_get_end_time(this.handle);
  }

  getTimeStep() {
    return this.M._bmi_get_time_step(this.handle);
  }

  getTimeUnits() {
    return this.#takeString(this.M._bmi_get_time_units(this.handle), 'get_time_units');
  }
}
