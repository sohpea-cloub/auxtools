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
`compiler/src/target.rs`; the arithmetic that differs is in
`compiler/src/compiler/versions.rs`.

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
0–100; HSV clamps from 516.1657; HCY does not clamp. HCY uses a green weight of 0.557
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
| `*(&x)` before 516.1668 | Reads or writes `*x`, dereferencing x's value: a "bad pointer" error unless x holds a pointer | Reads or writes x |
| `isnan()` or `isinf()` of a value not constant, in 516.1670 | Leaves out reading the test's flag; the server crashes | Reads the flag |
| Out-of-range constants in `&`, `\|`, `^`, `<<`, `>>` | Inconsistent, build-dependent 24-bit conversion | Uses the low 24 bits of the nearest 32-bit integer |
| `pick()` ending in a zero-weight entry, before 516.1689 | Jumps outside the proc if no earlier entry is selected | Selects the last entry whose weight is not a constant zero |
| `continue LABEL` to a labelled block holding a `do` ... `while`, inside a counted `for` loop | Pops the `for` loop's range (`PopN`) before jumping; the server crashes | Leaves the range on the stack |

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
logical assignment before 516.1673, indexed pointers from 516.1668, `*(&x)`
before 516.1668, the copied quirks, and zero-weight `pick()` endings before 516.1689.
`Snapshot::known_bugs` collects these across the project for `auxtools check`.

Each warning is a `diagnostic::Diagnostic` with its line and an id: one for each
compatibility difference, and one for each of BYOND's bugs (the `Byond` ids), which a
`byond_bugs` entry shares with its warning.

Warnings cover parameter-default division, safe-call receivers, logical assignment
before 516.1673, indexed pointer assignment from 516.1668, `*(&x)` before 516.1668,
out-of-range bit operands, zero-weight `pick()` endings before 516.1689, and
platform-sensitive angle/color folding.

Two copied quirks also warn:

- In 516.1648–516.1669 and 516.1671, constant `isnan`/`isinf` tests emit a result
  followed by `GetFlag`, returning the previous test flag. From 516.1670 a test inside
  another constant expression (`!isnan(1)`) folds with it, 516.1671 too.
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

## State carried between emitters

Some rules go by the source, not by what folding left, so the compiler compiles an
expression again with nothing folded, looks at the code and throws it away:
whether a folded `/type::var` default names another file (`names_other_file`) and
whether the value of `x.y ||= v` uses the cache (`unfolded_has`). Both run through
`Compiler::speculate`, which drops the code and restores every other field, the
warnings included, whether the trial compiled or not.

These fields carry something from one emitter to a later one. Each is set just
before the code that takes it; what would happen if other code came between:

- `regroup_root`: set before a return value, an argument or a parenthesised
  expression; taken by the next `emit_inner_expr`, which hands it to
  `binary_ops::emit` as `regroup_chain` when the expression is a binary
  operation, and drops it otherwise. Code compiled first would be regrouped
  instead.
- `place`: set before the place of an assignment that isn't `=` (`emit_place`,
  the statement form, `emit_conditional`); taken by the next `emit_inner_expr`,
  which then records no lookahead or compound-term line. The arms that compile a
  rewritten expression (`&(x)`, `*(p)`, `locate(X) in a in b`) and a place in
  parentheses compile the inner expression without it, so it records its lines;
  the statement form strips the parentheses first. Only a place written over
  several lines could differ, and DreamMaker refused the two such places tried.
- `safe_place`: set by `emit_conditional` in the debugger's code; taken by
  `emit_inner_expr` and given back just before `follow::emit_typed` runs on the
  place's follows, after the term, so nothing compiled for the term sees it.
- `list_elem_type`: set by `emit_inner_expr` just before `follow::emit_typed`,
  which takes it first.
- `skip_compound`: set for `return "[x]"` after its lookahead line; taken by the
  return value's `emit_inner_expr`. Such a value is never `.`, so it is always
  taken.
- `explicit_src`: set where a chain starts with `src` (`src.x`, `new src.a.b()`);
  cleared by the next instruction, the chain's first load. A call's arguments
  save and restore it: the receiver's `src.` is not theirs. An instruction
  emitted before the load would clear it, and the chain would reuse what a bare
  `x` left in the cache.
- `line_pending` (a default's line or a text initializer's lookahead line was
  recorded) and `read_on_pending` (a lookahead line was recorded): the next
  compound term records its own line. Any line record clears them, so they last
  until the next line, not the next expression. Without debug info nothing is
  recorded and they stay set, which changes nothing.
- `other_file`: the last line record named another file (a default's), or would
  have if the default hadn't folded; the next one names the proc's file again
  first. Without debug info it stays set, harmlessly.
- `mul_lhs`: set for the `*` that is the left operand of a `*`, in parentheses
  or not; taken by the next binary operation, which is that one, so it doesn't
  square. Until November 2026 it was set for any left operand, and a `*` inside
  a call, an index, a `? :` or a negation there lost its square.
- `condition_chain`: set while an `if` or `while` condition that is a `&&` or `||`
  compiles; taken by the first `&&` or `||`, the condition itself.

Others are set around one piece of code and cleared after it: `implicit_type`
(the value assigned to a typed place: the first implicit `new` or `astype()` in
it takes the type, as in DreamMaker, where `var/T/x = f(new)` makes a T),
`global_field` (set by field reads, cleared before a `||=` statement's value and
taken after it), `proc_macro_ok` (the proc of
`call(x, __PROC__)`, where DreamMaker allows `__PROC__` anywhere before 515.1644
too), `no_fold` and `live_only` (the trial compiles). `read_to` is reset at each
statement.

## Debugger code

`Compiler` compiles a proc from source (`compile_proc`) and what is typed into the debugger:
a body (`compile_body_in`) and a single expression (`compile_expr_*`). An expression is
compiled as the one-statement body `return EXPR`, without debug lines, so there is one path
for expressions and it is the one compared with DreamMaker. The debugger sends what
`is_body` takes for a body to the first and everything else to the second; both run in its
evaluation stub and return `list(result, params...)`.

The debugger's code is compiled as a global proc, the stub, over a tree of BYOND's own types
(parsed once), with `Compiler::debugger` set, and with `expr_scope` when the caller
has a paused frame to ask about. It differs from a proc's code in these places and no others:

| Where | Construct | In the debugger's code |
| --- | --- | --- |
| `compiler.rs:975`, `term.rs:220` | `__TYPE__`, `__PROC__`, `..()`, `.()` | Refused: the stub is not the paused proc and has no parent |
| `compiler.rs:1018`, `1042`, `procs.rs:355` | A bare name | A local of the body, a param (the frame's vars), then a var of `src`, then a global, by asking the scope; "undefined var" when nothing has it. A global is read through the world's list of them, or by the id the scope gives. Without a scope, a global |
| `procs.rs:927` | `vars` | A name like any other: BYOND's types declare it at the root as a const with no value |
| `term.rs:122`, `procs.rs:433` | `f()` | `src.f()` when the scope says `src` has it, else a global proc |
| `follow.rs:262` | `x.f()` | Called by name (`CallName`) when BYOND's types have no such proc: the world's types are not known, and a call by name finds verbs too |
| `follow.rs:504` | `global.x` | As a bare global; `global.vars` is the scope's list |
| `term.rs:88`, `compiler.rs:1330` | `/type/proc/name`, `/type::name()` | Found in the world through the scope |
| `compiler.rs:1321`, `fold.rs:219` | `/type::var`, `x::a::b` | Read when the code runs, not folded or resolved from a tree |
| `builtin_procs.rs:442`, `705`, `statements.rs:1385` | `astype(x)`, `istype(x)`, `var/T/x = new` | A declared type that BYOND's types lack is named as written and found when the code is assembled |
| `term.rs:811` | `/type{var = value}` | Refused: the instance would stay in the world's tables |
| `binary_ops.rs:309`, `fold.rs:605` | A bit operand beyond 32 bits | Left for the world to convert as it does |
| `assignment.rs:58` | `a?.b?.c \|\|= v`, `a?[i] &&= v` as a value | Compiled by older code in `follow.rs` (the `Safe` arms of `emit_typed`) that leaves a place: the proc path writes such an assignment as a value only for a plain `a?.b.c`, and a proc refuses the rest |

Until November 2026 the expression path was code of its own, from 2021, told apart by
whether the compile had a proc and a scope in 41 places, and never compared with DreamMaker.
What it did differently, for the record:

- No reason, both correct: `null` pushed as a value; `(x)` kept as a variable; `&&` and `||`
  chains with a target a pair of parentheses and constants tested when run; `? :` after `==`
  testing the flag twice; `?.` chains jumping to one target without saving the cache;
  assignments as values stored through the cache; a jump after a weighted `pick()`'s last
  value; `x ** 2` and `x * x` as `Pow` and `Mul`; a call of a proc BYOND's types have by
  name.
- Not what the world's code computes: `a + (b + c) + d` was not regrouped as DreamMaker
  regroups it (a list got two elements for one); a constant on the left of `&`, `|` or `^` was
  not stored as its 24 bits.
- Wrong: a built-in constant (`NORTH`, `TRUE`) was "undefined var" with a scope.
- Wrong in a body, which the expression had right: `..()` and `.()` compiled to calls of the
  stub; `vars` was null; `x = (ticks = 3)`, for a global, and `return a?.b?.c ||= v` were
  "expected l-value". Without a scope, an expression compiled `/type{...}`, which the
  debugger cannot assemble, and a body named the stub for `__PROC__`.

`compiler/tests/compile_body.rs` compiles expressions alone and as `return EXPR` and compares
the instructions.

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
