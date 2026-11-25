const assert = require("assert");
const { api, defines, launchFlags } = require("./buildflags");

assert.deepStrictEqual(defines(""), []);
assert.deepStrictEqual(defines("-DLOWMEMORYMODE"), ["LOWMEMORYMODE"]);
assert.deepStrictEqual(defines("-DA -DB=2  -DC"), ["A", "B=2", "C"]);
assert.deepStrictEqual(defines('-DMAP="tram station" -DTESTING'), ['MAP="tram station"', "TESTING"]);
assert.deepStrictEqual(defines("-D -X -DOK"), ["OK"]);

const flags = {
	current: () => ["TESTING", 'MAP="tram"'],
	onDidChangeFlags: () => ({ dispose() {} }),
	buildsWithFlags: (task) => task === "Build All" || task === "ss13buildflags: Build All",
};
const withExtension = (extension) => ({ extensions: { getExtension: (id) => (id === "ss13.ss13buildflags" ? extension : undefined) } });

(async () => {
	assert.strictEqual(await api(withExtension(undefined)), undefined);
	assert.strictEqual(await api(withExtension({ isActive: true, exports: {} })), undefined);
	assert.strictEqual(await api(withExtension({ isActive: true, exports: flags })), flags);
	assert.strictEqual(await api(withExtension({ isActive: false, activate: async () => flags })), flags);
	assert.strictEqual(await api(withExtension({ isActive: false, activate: async () => { throw new Error("no"); } })), undefined);

	assert.deepStrictEqual(launchFlags(flags, { preLaunchTask: "Build All" }), ["TESTING", 'MAP="tram"']);
	assert.deepStrictEqual(launchFlags(flags, { preLaunchTask: "ss13buildflags: Build All" }), ["TESTING", 'MAP="tram"']);
	assert.deepStrictEqual(launchFlags(flags, { preLaunchTask: "Build All (testing)" }), []);
	assert.deepStrictEqual(launchFlags(flags, {}), []);

	console.log("ok: buildflags");
})().catch((error) => {
	console.error(error);
	process.exit(1);
});
