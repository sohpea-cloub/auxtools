# Proc compiler

`dmasm::compiler::compile_proc` compiles a proc parsed by SpacemanDMM's
`dreammaker` crate into nodes that assemble to BYOND bytecode. It targets the
running world's BYOND build so the result can replace a proc through hot reload.

## Validation

The comparison harness compiles every proc in a codebase, assembles it against
DreamMaker's `.dmb`, and compares the resulting disassembly. Results from 2026-11-10:

| Codebase commit | BYOND | Procs | Identical | Equivalent | Known differences |
| --- | --- | --- | --- | --- | --- |
| cmss13 `f712f7b` | 516.1687 | 28,330 | 19,456 | 8,872 | 2 |
| tgstation `928bb812` | 516.1685 | 66,712 | 43,255 | 23,422 | 35 |
| goonstation `3e27edf` | 516.1687 | 44,522 | 29,643 | 14,875 | 2 |
| tgstation `addb78ce0b` | 516.1659 | 65,478 | 42,499 | 22,944 | 35 |
| tgstation `6a18424329` | 515.1647 | 58,937 | 38,860 | 20,041 | 36 |
| tgstation `7faadfdc16` | 515.1637 | 58,171 | 38,343 | 19,791 | 37 |

“Identical” includes debug information. “Equivalent” excludes debug information
and applies only the encoding normalizations below. The remaining mismatches are
known differences described here.

The harness also replaces proc bytecode in the `.dmb` and runs codebase unit tests:
150 for cmss13, 486 for tgstation and 22 for goonstation, with the same results as
DreamMaker builds. Differential fuzzing compared 1,536 worlds of 80 generated procs.
The offline `.dmb` tooling is not included in this repository.

## Target BYOND build

Set `ProcSource::byond`, `compile_expr_for` or `reload::ParseOptions::byond` to the
running world's build. `ByondVersion::read_as` configures version-specific parsing.
Every build at which dmasm does something differently is a predicate on `ByondVersion` in
`src/target.rs`; the arithmetic that differs is in `src/compiler/versions.rs`.

Probes covered 515.1637, 515.1647, 516.1659, 516.1685, 516.1688 and 516.1689. Additional probes
located behavior changes across 515.1637–516.1660, and in 516.1664, 1668–1672, 1676,
1681 and 1684.

- **Builtins and syntax:** 515 uses `Sin`, `Cos`, `Tan` rather than their `*2`
  instructions. Calls to unavailable builtins resolve as global procs. Unsupported
  `alist()` and key/value iteration require 516. `__PROC__` requires 515.1644.
- **Integer literals:** before 515.1644, integer literals above 2147483647 clamp to
  that value, including hexadecimal and octal literals. Decimal-point and exponent
  forms such as `3000000000.0` and `3e9` are unaffected.
- **Folding:** 515 does not fold `ceil`, `floor`, `round`, `trunc` or `fract`.
  516 also folds `isnan` and `isinf`, with the flag-register quirk below.

## Constant folding and platform differences

The compiler follows Linux DreamMaker. It has no build-platform option.

### Math

515 converts trigonometric inputs directly to radians. 516 normalizes angles and
reduces by quadrants, producing exact axis values for sin/cos. For tan, it reduces
to at most 45 degrees, computes in f64, rounds to f32, then applies reciprocal and
sign adjustments. These steps affect the result and must retain their order.
`arctan(x, -0)` treats the second argument as zero. `%%` folds with f64 `fmod`.

From 515.1644, Linux DreamMaker rounds inverse-trig radians to f32 before converting
to degrees. Windows uses f64 throughout. Some constants differ by one f32 rounding
step (`arctan(2)`: 63.43495 on Linux, 63.434948 on Windows). DreamDaemon uses the
Linux angle result on both platforms. The compiler warns when Windows folding differs.

### Colors

`rgb()` folds in every color space. RGB and alpha channels truncate to integers;
HSV, HSL and HCY channels round half up. HSL clamps saturation and luminance to
0–100; HSV clamps from 516; HCY does not clamp. HCY uses a green weight of 0.557
for `rgb()` and 0.587 for `rgb2num()`.

Linux DreamMaker from 515.1647 normalizes percentage inputs in f32, computes in f64,
then rounds channels through f32 before integer rounding. Windows and 515.1637 use
different intermediate precision. Channels near a half-integer boundary can differ
by one (276 of 31,752 sampled colors differed between Linux and Windows on
516.1688). The compiler uses Linux rounding
and warns near those boundaries. Before 515.1647, it leaves such colors for runtime.

`rgb2num()` folds RGB results to integer lists. Inputs include short and long hex
colors with optional alpha, 22 case-insensitive color names, and null (white).
Spaces other than 1, 2 and 3 select RGB. In a condition, the result is known to be
true in any color space without constructing the list. HSV/HSL/HCY results remain runtime expressions
because their folded precision differs between builds and platforms.

Runtime color conversion follows the local DreamDaemon platform. Runtime angle
conversion follows Linux on both platforms. Other platform differences found in
numeric, text, statement and builtin sweeps were limited to out-of-range bit operands.

## Equivalent encodings

The comparison normalizes:

- Static versus dynamic proc calls. DreamMaker chooses based on compilation order;
  dmasm uses dynamic calls because that order is unknown.
- Label names and switch target encodings, comparing resolved destinations instead.
- Redundant cache loads of plain `src`, arguments or locals.

Cache reuse through fields is behaviorally significant and is not normalized.
For example, after `a.b.f()` changes `a.b`, DreamMaker still uses the cached old
receiver for `a.b.x`. dmasm matches these reuse and invalidation rules.

## Known differences

| Source or case | DreamMaker | dmasm |
| --- | --- | --- |
| Parameter default `a.b/10` without spaces | Parses a path and omits division; `x/-10` becomes subtraction | Performs division |
| `a?.f(b?.x)?.y = v` | Calls f on the receiver left by argument evaluation | Restores f's receiver |
| `a?.b \|\|= v` or `&&=` before 516.1673 | Omits null checks; with multiple `?.` accesses, reads and writes the first object's variable | Preserves null checks and the final target |
| `/T/name` subtype block where name is a proc of T | Appends the block to the proc | Parses a subtype |
| Empty proc at a file's end | Names the next included file in debug info | Uses the proc's source file |
| `*p[i] = x` from 516.1668 | Drops the index and writes through the cache | Writes the indexed element |
| Out-of-range constants in `&`, `\|`, `^`, `<<`, `>>` | Inconsistent, build-dependent 24-bit conversion | Uses the low 24 bits of the nearest 32-bit integer |
| `pick()` ending in a zero-weight entry, before 516.1689 | Jumps outside the proc if no earlier entry is selected | Selects the last entry whose weight is not a constant zero |

Hexadecimal literals longer than eight digits and octal literals exceeding i32
are rejected. DreamMaker's conversion is unusual (`0x100000000` becomes `2**60`)
and is not implemented.

## Debug lines

Debug builds emit `DbgFile` and `DbgLine` records to match DreamMaker.
Breakpoints use these records; a line without one moves to the next recorded
line.

- Statements record their lines, including uninitialized `var`, `var/const`
  and `set`. `else`, `;`, empty macro expansions, comments and lines removed
  by `#if` have no statement records.
- Before the first statement, record the first nonempty line after the
  header. This includes empty macro expansions, `;`, an opening body brace
  and a multiline macro call's name. Empty bodies use the next nonempty
  line, usually the next proc's header (`ProcValue::first_line`).
- Parameter defaults record the header's line. A return type (`as num`) records
  it once more after the defaults.
- DreamMaker also records source lookahead before compiling the current
  code. The parser supplies `ProcValue::next_lines`, `end_lines` and
  `operator_lines` to reproduce these records:
  - Initializers starting with interpolated text (`var/s = "[x]"`) record
    the next nonempty line before the value, then the text's end line.
    Directives and empty macros count; comments do not. `return "[x]"`
    records that next line before its own. These records also appear at
    switch-case entries and loop exits when the statement occurs there.
  - The value var in `for (var/k, v in L)` records the body's first line,
    then the loop's line again.
  - Multiline named-argument calls and `if`/`while` conditions record their
    closing parenthesis. Interpolated text records its end line. Binary
    operators, `?` and dot follows on later lines record their lines; the
    next term then records its own, even if unchanged. Expression statements
    record such operator lines before the statement line.

Debug-line compatibility measured on 2026-11-13 using the corpora above:

| Codebase | Procs | Same, start | Same, first-line rules | Same, all rules |
| --- | --- | --- | --- | --- |
| cmss13 | 28,330 | 26,311 | 27,291 | 27,949 |
| tgstation | 66,712 | 58,470 | 64,363 | 66,166 |
| goonstation | 44,522 | 42,576 | 43,285 | 44,341 |
| tgstation 515.1637 | 58,171 | 51,780 | 56,395 | 57,697 |
| tgstation 515.1647 | 58,937 | 52,397 | 57,119 | 58,459 |
| tgstation 516.1659 | 65,478 | 57,575 | 63,278 | 64,953 |

Remaining differences:

| Case | Procs (cmss13, tgstation, goonstation) | DreamMaker | dmasm |
| --- | --- | --- | --- |
| A proc defined by a macro (`GLOBAL_LIST_INIT`, goonstation's `HELP_MESSAGE_OVERRIDE`) | 188, 290, 25 | Uses lines past the macro's use | Uses the macro's line |
| The last proc of an included file | 9, 39, 22 | Names the `.dme`'s `#include` line between its own, or (empty) the next file's first line | Names its own file |
| Other code over several lines: statements a macro expands to (`LAZYADD`, `DB_VIEW`), conditions of several operators continued with `\`, list entries assigned over several lines, ternaries in text | 184, 217, 130 | Records further lines it has read on to, blank ones in a macro's expansion too | Records the lines above |

These gaps can move breakpoints after reload. When DreamMaker records a
lookahead line early, its bytecode can stop a statement sooner.

## Warnings

`CompiledProc::warnings` reports intentional behavior differences and significant
DreamMaker quirks retained for compatibility. Reload forwards these diagnostics.

`CompiledProc::byond_bugs` contains warnings about bugs affecting DreamMaker
builds, excluding dmasm workaround details and compatibility-only differences.
It includes the bugs below, parameter-default division, safe-call receivers,
logical assignment before 516.1673, indexed pointers from 516.1668, the copied
quirks, and zero-weight `pick()` endings before 516.1689.
`Snapshot::known_bugs` collects these across the project for `auxtools check`.

Warnings cover parameter-default division, safe-call receivers, logical assignment
before 516.1673, indexed pointer assignment from 516.1668, out-of-range bit operands,
zero-weight `pick()` endings before 516.1689, and platform-sensitive angle/color folding.

Two copied quirks also warn:

- In 516.1648–516.1669 and 516.1671, constant `isnan`/`isinf` tests emit a result
  followed by `GetFlag`, returning the previous test flag.
- Before 516.1685, changing a list element through `?.` with an RHS that uses
  the cache resolves the list from the cache value left by the RHS.

After `a.b.f()`, DreamMaker keeps `a.b` cached: a later `a.b.x` in the same
flow uses the old object even if `f()` replaced `a.b`. dmasm preserves this
behavior without warning because the pattern is common and only matters when
the call replaces the field. Read the field into a local when a call may replace it.

The following bugs affect both DreamMaker builds and reloads
(`compiler/known_bugs.rs`):

- `continue LABEL` to a labelled `do` ... `while` inside a counted `for` loop
  crashes the server.
- Nested list loops separated by `try`: catching an inner-loop error makes
  the outer loop use the inner iterator. It can end early, loop forever or crash.
- `operator<<=` and `operator>>=`: the result is discarded. `operator++` and
  `operator--`: as a statement, called without an argument and the result
  discarded. `operator**` on a type with `operator*`: `a ** 2` calls `operator*`.
- `operator<=>` from 516.1674: `list.Find()` and `list ^ x` call it, and from
  516.1689 take 0 for a match. `operator~=` and `operator~!` in 516.1674 to
  516.1688: the result is discarded.
- `Uncross()` of an area from 516.1683: the result is ignored.
- `*p[i]` read from 516.1668, as for its assignment above.

Debug-location and encoding-only differences do not warn. In the validation corpora,
most warnings are tgstation parameter-default divisions; older builds also expose
the safe-list-assignment quirk.

## Compiler compatibility rules

These rules affect generated code and are covered by focused tests:

- User const variables, including local consts, occupy global slots. Standalone
  reads access the slot; larger constant expressions fold the value. Built-in
  constants such as `NORTH` are emitted directly.
- Static locals are block-scoped. Their scope (`StaticScope::Proc`) is the proc's path
  and which declaration of the name in that proc it is, from 0.
- Static/const access through `x::a::b` does not evaluate receivers preceding b.
  `/type::proc()` selects that type's override. `:` lookup searches subtypes depth
  first; unknown receiver calls use the first declaration's display name.
- After the first for loop, label, switch or pick(), uninitialized variable
  declarations explicitly assign null.
- For loops, ternary else branches and discarded logical operands invalidate cache
  knowledge. Labels retain it only with one incoming edge; switch cases sharing a
  target count as one, while fallthrough after `return` still counts.
- Assignment statements invalidate cached field chains, except list-entry writes,
  most declarations and assignments compiled as values. `var/L[] = new()` is an
  exception among declarations. Calls outside the cache invalidate chains; receiver
  calls, `..()` and spawn preserve them. `x.y ||= v` resolves its target before testing.
- Logical assignments resolve fields/list entries once, saving the cache across
  RHS code that needs it. Static/const reads through `.` and `global.x` do not
  require a save; `::` accesses do. A list-indexing RHS also saves the target key.
- `del(x)` clears a writable target before deletion; `del src` ends the proc.
  `del(world)` only deletes.
- Empty arguments use `PushVal null`; explicit null uses `GetVar null`.
  A `null = x` associative key becomes the text `"null"`.
- `var/L[n]` emits `n; EmptyList`; multidimensional declarations use `new /list`.
  In `var/x = a in L`, L is the declaration's input clause. Statement `x = a in L`
  parses as `(x = a) in L`.
- In return values, arguments and parentheses, regroup associative `+ * & | ^`
  expressions before folding. Fold math builtins except min/max; reversed clamp
  bounds are swapped. `X * X` uses PowSquare for supported values, excluding
  interpolated strings and fractions.
- Binary bit operations clamp operands to i32 and retain 24 bits. `~` converts
  through u32, with negative and overflow behavior covered by tests.
- Safe assignment emits nested null checks across fields, calls and indexes.
  Value expressions use SetCacheJmpIfNull; statement targets use SetCachePopJmpIfNull.
- PickSwitch always jumps, preserving cache knowledge for its first arm.
- Spawn without a block applies to the next statement. Its code retains enclosing
  loop targets. Labeled break exits the block; try-block exits emit Catch/TryJmp.
- `for (var/datum/T/x)` enumerates all datums of type T. With `in world`, it
  enumerates atoms and checks their type, so finds no datums.
  `for (var/list/T/x in L)` checks x as a T.
- `for (var/T/k, v in L)` filters and type-checks the key as in
  `for (var/T/x in L)`.
- Input has four arguments, padded with null, and a built-in or stack-based `in`
  source. `findlasttext` defaults to start 0 and end 1.
- Absolute-path parameters and `...` do not allocate parameter slots.
- Prefabs use instance-table entries with init procs. Order variable assignments
  by string ID and emit source locations before computed values. Assembly resolves
  the instance through `AssembleEnv::get_instance`.

## Debugger code

`Compiler` serves three callers. `compile_proc` compiles a proc from source: `proc` holds
what it can see (the tree, its type, its locals). `compile_body_in` compiles statements typed
into the debugger: `proc` is a made-up global proc over a tree of BYOND's own types, and
`expr_scope`, when the caller has a paused frame, says what the world has. `compile_expr_*`
compiles one expression typed into the debugger: no `proc`, and `expr_scope` or not. The
debugger sends what `is_body` takes for a body to the second and everything else to the
third, and both run in its evaluation stub and return `list(result, params...)`.

Only the proc path is compared with DreamMaker. The emitters tell the callers apart by
`proc.is_some()`, `proc.is_none()` and `expr_scope`, in 41 places on `v4` at `b782411`
(besides the two entry points, the field and its initial value). Each is one of:

- **A**: the debugger's situation, to keep whichever path compiles its code. It has no
  parsed tree of the world: names, globals, procs and types are found in the running world
  or left to it. Its code runs in a stub that is no proc of the world's.
- **B**: no such reason. The expression path emits other instructions than a body or a proc
  does for the same text, or one of them refuses what the other takes.
- **C**: the test decides nothing: no input reaches it with the other answer.

| Where | Construct | Class | Bare expression, against body and proc |
| --- | --- | --- | --- |
| `compiler.rs:910` | `__TYPE__`, `__PROC__` | A | Refused in the debugger's code: the stub is not the paused proc. Tested by proc and no scope, so a body without a scope names the made-up proc |
| `compiler.rs:953`, `977` | A bare name | A | A var of `src`, then a global, by asking the world; an error when neither. Without a scope, a global |
| `compiler.rs:1089` | Folding, and the lines an expression records | B | Folds as a proc does, but not what an assignment writes to, and does not forget the cache where `&&` or `\|\|` folds. Both correct. An expression has no lines |
| `compiler.rs:1211` | `&(x)`, `*(p)` | B | Goes with `term.rs:23`: needed once parentheses give a value. Both correct |
| `compiler.rs:1244`, `term.rs:269` | `src.a.b` after a chain from `src` | B | The proc path loads the cache again for a chain written with `src.`. Both correct |
| `compiler.rs:1268` | `/type::var` | A | Read from the world when the code runs, not from a tree. Tested by scope: a body without one takes BYOND's types and refuses the world's |
| `compiler.rs:1277` | `/type::proc()` | A | Found in the world through the scope |
| `compiler.rs:1312` | An argument left empty, `f(a, , b)` | B | Same instruction either way today. Goes with `term.rs:31` |
| `compiler.rs:1327` | `var/L[n]` | C | A declaration: no expression has one |
| `term.rs:23` | `(x)` | B | Stays a variable: `(x).y` reads through `x`, where a proc puts `x` on the stack and regroups what is inside (see `binary_ops.rs:171`). Both correct |
| `term.rs:31` | `null` | B | `PushVal null`, where a proc reads the variable `null`. Both correct |
| `term.rs:91`, `125` | A proc path; a bare call | A | Found in the world; `f()` is `src.f()` when `src` has it |
| `term.rs:222` | `..()`, `.()` | A, B | Refused in an expression: the stub has no parent and is not the paused proc. Tested by proc, so a body takes them and calls the stub: the body is wrong |
| `term.rs:491` | `pick(a; x, b; y)` | B | A jump to the end after the last value too. Both correct |
| `term.rs:816` | `/type{var = value}` | A | Refused: the instance would stay in the world's tables. Tested by scope: without one it compiles and fails to assemble |
| `binary_ops.rs:171` | `a + (b + c) + d` | B | Grouped as written, where DreamMaker moves the parentheses in a return value, an argument or parentheses: `a + ((b + c) + d)`. The value differs for lists and in the last bit of a float: the expression is not what the world's code computes |
| `binary_ops.rs:188` | `(a \|\| b) \|\| c` | B | One jump target a pair of parentheses, where a proc has one for the chain. Both correct |
| `binary_ops.rs:200` | `1 && x`, `x \|\| 0 \|\| y` | B | Tests the constant when the code runs, where a proc drops it. Both correct |
| `binary_ops.rs:260`, `276` | `x ** 2`, `x ** -1`, `x * x` | B | `Pow` and `Mul`, where a proc has `PowSquare` and `PowNegativeOne`. The same for numbers |
| `binary_ops.rs:311` | `K & x`, K a constant | B | The number as written, where a proc stores its 24 bits. They differ when K does not fit: `16777217 & x` |
| `binary_ops.rs:319`, `fold.rs:608` | A bit operand beyond 32 bits | A | Left for the world to convert as it does. Tested by scope |
| `binary_ops.rs:325` | `K * x`, K a const var | B | No difference without a tree: the code's consts are not known |
| `ternary.rs:30` | `a == b ? x : y` | B | Fetches the test flag and tests the value, where a proc jumps on the flag. Both correct |
| `follow.rs:88` | `a?.b?.c`, `a?[i]`, `a?.b++` | B | Its own code from 2021 unless `!`, `-` or `~` ends the chain: one target for the chain and the cache not saved. Both correct |
| `follow.rs:267` | `x.f()` | A, B | Called by name (`CallName`), which finds verbs too: the world's types are not known. A body does so only with a scope, and only for names BYOND's types lack; an expression for every name. Both correct |
| `follow.rs:296` | `x.f()`, the receiver in the call's operand | B | Not reached by an expression, which calls by name |
| `follow.rs:423` | `x::name` | B | A copy of what the proc path does for a value of unknown type. Same instructions |
| `follow.rs:524` | `global.x` | A | Through the world's list of globals, or the id it gives |
| `assignment.rs:250` | `(L[i] = v)`, `(*p = v)`, `a.b \|\|= v` as values | B | `SetVarExpr` through the cache, where a proc copies the value and stores it (`PushTop`, `ListSet`). Both correct. The proc path refuses a global reached through the scope's list: `x = (ticks = 3)` in a body is "expected l-value", which is wrong |
| `assignment.rs:515` | The cache saved around `\|\|=`'s value | C | Inside what `assignment.rs:250` guards |
| `builtin_procs.rs:442`, `708`, `statements.rs:1395` | `astype(x)`, `istype(x)`, `var/T/x = new` | A | A type of the world is named and found when the code is assembled. Tested by scope |
| `fold.rs:219` | `x::a::b`, a const | A | Not folded: the world's types are not known. Tested by scope |
| `procs.rs:353`, `433` | Names and bare calls in a body | A | Left to the scope, not to the made-up proc's type. Tested by scope |

Two more differences come from the expression path having no tree at all: a built-in
constant (`NORTH`, `TRUE`) is a name to look up, so "undefined var: NORTH" with a scope, where
a body pushes 1; and a body reads `vars` as the root type's const, null, where an expression
reads `src.vars`. Another 27 places read `proc` and do nothing without one.

## Unsupported forms

Compilation returns an error for unsupported constructs, including savefile-to-
savefile reads (`S["a"] >> S["b"]`), standalone `load_resource()` and nonconstant
switch cases. The caller must retain existing bytecode or reject the reload.

Assembly also needs resources, strings, procs and prefab instances available through
its environment. Resources must already exist in the world's `.rsc`; adding runtime
table entries is the hot reload caller's responsibility.

## Parser dependency

`Cargo.toml` uses [auxtools/SpacemanDMM](https://github.com/auxtools/SpacemanDMM)
for parser and preprocessor fixes required by compilation: distinct alist terms,
empty arguments, for-in type clauses, macro stringification, source-location macros,
raw strings, declaration flags, pragma handling and absolute parameter metadata.
