// End-to-end test: load the adapter wasm, dlopen the leaky-bucket side module,
// and drive it through a full BMI lifecycle. Run via ./build.sh --test
import { readFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
import createBmiModule from '../pkg/bmi.mjs';
import { BmiRuntime } from '../js/bmi.mjs';

const rt = await BmiRuntime.load(createBmiModule);

// Stage the model "shared object" and its config file into MEMFS.
rt.writeFile(
  '/models/leaky_bucket.wasm',
  await readFile(new URL('./model.wasm', import.meta.url))
);
rt.writeFile('/config/bucket.cfg', 'initial_storage=10.0\nleak_rate=0.1\n');

const model = rt.openModel('bucket', '/models/leaky_bucket.wasm', 'register_bmi_leaky_bucket');
model.initialize('/config/bucket.cfg');

assert.equal(model.getComponentName(), 'Leaky Bucket (wasm test model)');
assert.deepEqual(model.getInputVarNames(), [
  'atmosphere_water__precipitation_leq-volume_flux',
]);
assert.deepEqual(model.getOutputVarNames(), [
  'bucket__water_storage',
  'bucket__outflow',
  'bucket__iterations',
]);

const info = model.getVarInfo('bucket__water_storage');
assert.equal(info.type, 'double');
assert.equal(info.units, 'm');
assert.equal(info.itemsize, 8);
assert.equal(info.location, 'node');

assert.equal(model.getStartTime(), 0);
assert.equal(model.getEndTime(), 86400);
assert.equal(model.getTimeStep(), 3600);
assert.equal(model.getTimeUnits(), 's');
assert.equal(model.getCurrentTime(), 0);

// initial_storage=10, leak_rate=0.1, precip=1e-4 m/s over a 3600 s step:
// storage = (10 + 0.36) * 0.9 = 9.324
model.setValue('atmosphere_water__precipitation_leq-volume_flux', [1e-4]);
model.update();

const storage = model.getValue('bucket__water_storage');
assert.ok(storage instanceof Float64Array);
assert.ok(Math.abs(storage[0] - 9.324) < 1e-9, `storage was ${storage[0]}`);

const outflow = model.getValue('bucket__outflow');
assert.ok(outflow instanceof Float32Array);
assert.ok(Math.abs(outflow[0] - 1.036 / 3600) < 1e-6, `outflow was ${outflow[0]}`);

const iters = model.getValue('bucket__iterations');
assert.ok(iters instanceof Int32Array);
assert.equal(iters[0], 1);
assert.equal(model.getCurrentTime(), 3600);

// Run to the end of the simulation.
model.setValue('atmosphere_water__precipitation_leq-volume_flux', [0]);
model.updateUntil(model.getEndTime());
assert.equal(model.getCurrentTime(), 86400);
assert.equal(model.getValue('bucket__iterations')[0], 24);

// Errors surface as exceptions with the model's message, not aborts.
assert.throws(() => model.getVarInfo('nope') && model.getValue('nope'), /get_value/);

// A second instance of the same library is independent state.
const model2 = rt.openModel('bucket2', '/models/leaky_bucket.wasm', 'register_bmi_leaky_bucket');
model2.initialize('/config/bucket.cfg');
assert.equal(model2.getValue('bucket__iterations')[0], 0);
assert.ok(Math.abs(model2.getValue('bucket__water_storage')[0] - 10.0) < 1e-12);
model2.close();

model.finalize();
model.close();

console.log('bmi-wasm end-to-end test passed');
