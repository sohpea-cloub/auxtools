# Proc compiler

`dmasm::compiler::compile_proc` compiles a proc that SpacemanDMM's `dreammaker` crate has
parsed. The result is a list of nodes that assemble to BYOND bytecode. The compiler targets the
running world's BYOND build, so that hot reload can replace a proc with the result.

## Validation

A comparison harness compiles every proc in a codebase and assembles it against DreamMaker's
`.dmb`. Then it compares the disassembly with DreamMaker's. Results from 2026-11-10:

| Codebase commit | BYOND | Procs | Identical | Equivalent | Known differences |
| --- | --- | --- | --- | --- | --- |
| cmss13 `f712f7b` | 516.1687 | 28,330 | 19,456 | 8,872 | 2 |
| tgstation `928bb812` | 516.1685 | 66,712 | 43,255 | 23,422 | 35 |
| goonstation `3e27edf` | 516.1687 | 44,522 | 29,643 | 14,875 | 2 |
| tgstation `addb78ce0b` | 516.1659 | 65,478 | 42,499 | 22,944 | 35 |
| tgstation `6a18424329` | 515.1647 | 58,937 | 38,860 | 20,041 | 36 |
| tgstation `7faadfdc16` | 515.1637 | 58,171 | 38,343 | 19,791 | 37 |

"Identical" procs match DreamMaker's, debug information included. "Equivalent" procs match
when the debug information is left out and the encodings below are normalized. All other
procs differ in the known ways that this document describes.

The harness also puts the compiled bytecode into the `.dmb` and runs the codebase's unit tests:
150 for cmss13, 486 for tgstation and 22 for goonstation. The results were the same as with
DreamMaker's builds. Differential fuzzing compared 1,536 worlds of 80 generated procs each.

The codebase harness, its bytecode replacement and the fuzzer are not in this repository. The
`compare_dreammaker` example is. It compares procs in the same way, for the small worlds in
`compiler/tests/dreammaker/`. [Comparing with DreamMaker](README.md#comparing-with-dreammaker)
describes how to run it.

## Target BYOND build

Set `ProcSource::byond`, `compile_expr_for` or `reload::ParseOptions::byond` to the running
world's build. `ByondVersion::read_as` sets up parsing for that build. `compiler/src/target.rs`
has a check on `ByondVersion` for each build where dmasm's output changes.
`compiler/src/compiler/versions.rs` has the arithmetic that differs between builds.

Probes covered 515.1637, 515.1647, 516.1659, 516.1685, 516.1688 and 516.1689. More probes found
where behavior changes in 515.1637–516.1660, and in 516.1664, 1668–1672, 1676, 1681 and 1684.

- **Builtins and syntax.** 515 uses `Sin`, `Cos` and `Tan` instead of their `*2` instructions.
  A call to a builtin that the build lacks compiles as a call to a global proc. `alist()` and
  key/value iteration need 516. `__PROC__` needs 515.1644.
- **Integer literals.** Before 515.1644, an integer literal above 2147483647 becomes
  2147483647. This includes hexadecimal and octal literals. Literals with a decimal point or an
  exponent, such as `3000000000.0` and `3e9`, don't change.
- **Folding.** 515 doesn't fold `ceil`, `floor`, `round`, `trunc` or `fract`. 516 also folds
  `isnan` and `isinf`, with the flag quirk described under [Warnings](#warnings).

## Constant folding and platform differences

The compiler folds constants as Linux DreamMaker does. It has no option for the build platform.

### Math

515 converts the inputs of trigonometric functions straight to radians. 516 normalizes the
angle and reduces it by quadrants, which gives exact values on the axes for sin and cos. For
tan, 516 reduces the angle to 45 degrees or less, computes in f64 and rounds to f32. Then it
takes the reciprocal and sets the sign as needed. Each of these steps changes the result, and
they must stay in this order. `arctan(x, -0)` treats the second argument as zero. `%%` folds
with the f64 `fmod`.

Inverse trigonometric functions fold in f64 and round once to f32. DreamMaker on Windows, and
on Linux before 515.1644, calls the f64 functions. From 515.1644, Linux DreamMaker calls the f32
functions of glibc. Before glibc 2.41, they return the x87 result without rounding it, so the
folded value is the same. From glibc 2.41, they round the radians to f32, and some constants
differ by one f32 rounding step. For example, `arctan(2)` is 63.434948, and 63.43495 with glibc
2.41. This was checked with glibc 2.35, 2.36, 2.39 and 2.41. The compiler warns when glibc 2.41
would give another value.

### Colors

`rgb()` folds in every color space. RGB and alpha channels are truncated to integers. HSV, HSL
and HCY channels are rounded, with halves rounded up. HSL clamps saturation and luminance to
0–100. HSV clamps them from 516.1657. HCY doesn't clamp. HCY uses a green weight of 0.557 for
`rgb()` and 0.587 for `rgb2num()`.

From 515.1647, Linux DreamMaker normalizes percentage inputs in f32 and computes in f64. Then
it rounds each channel to f32 before it rounds to an integer. Windows and 515.1637 use another
precision in between. So a channel near a half-integer can differ by one: 276 of 31,752 sampled
colors differed between Linux and Windows on 516.1688. The compiler rounds as Linux does and
warns near those boundaries. For builds before 515.1647, it leaves such colors to be computed
at runtime.

`rgb2num()` folds RGB results to lists of integers. Its input can be a short or long hex color
with or without alpha, one of 22 color names in any case, or null, which is white. A color space
other than 1, 2 and 3 selects RGB. In a condition, the result is always true in any color space,
so the compiler doesn't build the list. HSV, HSL and HCY results are computed at runtime,
because their folded precision differs between builds and platforms.

At runtime, color conversion follows the platform that DreamDaemon runs on. On Linux, angle
conversion follows the glibc that DreamDaemon runs with, as DreamMaker does. Windows DreamDaemon
rounds the radians to f32 in every build, as checked under Wine. Sweeps of numeric, text, statement and builtin code found no
other platform differences, except for bit operations on out-of-range operands.

## Equivalent encodings

The comparison normalizes these differences:

- Static and dynamic proc calls. DreamMaker chooses one by the order in which it compiles the
  procs. dmasm doesn't know that order, so it always uses dynamic calls.
- Label names and the encoding of switch targets. The comparison compares the destinations
  instead.
- Cache loads of plain `src`, arguments or locals that are not needed.

The comparison doesn't normalize cache reuse through fields, because it changes behavior. For
example, after `a.b.f()` changes `a.b`, DreamMaker still uses the old object from the cache for
`a.b.x`. dmasm follows the same rules for when the cache is reused and when it is cleared.

## Known differences

| Source or case | DreamMaker | dmasm |
| --- | --- | --- |
| Parameter default `a.b/10` without spaces | Parses a path and leaves out the division; `x/-10` becomes a subtraction | Divides |
| `a?.f(b?.x)?.y = v` | Calls f on the object that the argument's evaluation left in the cache | Calls f on its own object |
| `a?.b \|\|= v` or `&&=` before 516.1673 | Leaves out the null checks; with several `?.` accesses, reads and writes the variable of the first object | Keeps the null checks and the last target |
| `/T/name` subtype block where name is a proc of T | Adds the block to the proc | Parses a subtype |
| Empty proc at the end of a file | Names the next included file in the debug information | Names the proc's source file |
| `*p[i] = x` from 516.1668 | Drops the index and writes through the cache | Writes the indexed element |
| `*(&x)` before 516.1668 | Reads or writes `*x`, so it dereferences x's value: a "bad pointer" error unless x holds a pointer | Reads or writes x |
| `isnan()` or `isinf()` of a non-constant value, in 516.1670 | Doesn't read the test's flag; the server crashes | Reads the flag |
| Out-of-range constants in `&`, `\|`, `^`, `<<`, `>>` | Converts to 24 bits in ways that differ between builds and cases | Uses the low 24 bits of the nearest 32-bit integer |
| `pick()` that ends in an entry with weight zero, before 516.1689 | Jumps outside the proc if no earlier entry is selected | Selects the last entry whose weight is not a constant zero |
| `continue LABEL` to a labelled block that holds a `do` ... `while`, inside a counted `for` loop | Pops the `for` loop's range (`PopN`) before it jumps; the server crashes | Leaves the range on the stack |

dmasm rejects hexadecimal literals of more than eight digits, and octal literals that don't fit
in an i32. DreamMaker converts them in an unusual way, which dmasm doesn't copy: `0x100000000`
becomes `2**60`.

## Debug lines

In debug builds, the compiler emits `DbgFile` and `DbgLine` records as DreamMaker does.
Breakpoints use these records. A breakpoint on a line without a record moves to the next line
that has one.

- Statements record their lines. This includes `var` without a value, `var/const` and `set`.
  `else`, `;`, empty macro expansions, comments and lines that `#if` removes don't record a
  line.
- Before the first statement, the proc records the first nonempty line after its header. That
  line can be an empty macro expansion, a `;`, the body's opening brace, or the name of a macro
  call over several lines. An empty body records the next nonempty line, usually the next
  proc's header (`ProcValue::first_line`).
- Parameter defaults record the header's line. A return type (`as num`) records it once more,
  after the defaults.
- DreamMaker also records lines that it reads ahead, before it compiles the current code. The
  parser gives `ProcValue::next_lines`, `end_lines` and `operator_lines` so that dmasm can
  record the same lines:
  - An initializer that starts with interpolated text (`var/s = "[x]"`) records the next
    nonempty line before the value, and then the line where the text ends. Directives and empty
    macros count as lines here, but comments don't. `return "[x]"` records that next line before
    its own line. These records also appear at switch-case entries and loop exits when the
    statement is there.
  - In `for (var/k, v in L)`, the value variable records the body's first line, and then the
    loop's line again.
  - A call with named arguments over several lines, and an `if` or `while` condition over
    several lines, record the line of the closing parenthesis. Interpolated text records the
    line where it ends. A binary operator, `?` or `.` at the start of a later line records that
    line. The next term then records its own line, even if the line is the same. An expression
    statement records such operator lines before the statement's line.

Debug lines compared with DreamMaker on 2026-11-13, on the same codebases as above:

| Codebase | Procs | Same, start | Same, first-line rules | Same, all rules |
| --- | --- | --- | --- | --- |
| cmss13 | 28,330 | 26,311 | 27,291 | 27,949 |
| tgstation | 66,712 | 58,470 | 64,363 | 66,166 |
| goonstation | 44,522 | 42,576 | 43,285 | 44,341 |
| tgstation 515.1637 | 58,171 | 51,780 | 56,395 | 57,697 |
| tgstation 515.1647 | 58,937 | 52,397 | 57,119 | 58,459 |
| tgstation 516.1659 | 65,478 | 57,575 | 63,278 | 64,953 |

The remaining differences:

| Case | Procs (cmss13, tgstation, goonstation) | DreamMaker | dmasm |
| --- | --- | --- | --- |
| A proc that a macro defines (`GLOBAL_LIST_INIT`, goonstation's `HELP_MESSAGE_OVERRIDE`) | 188, 290, 25 | Records lines after the macro's use | Records the macro's line |
| The last proc of an included file | 9, 39, 22 | Records the `.dme`'s `#include` line among the proc's own lines. For an empty proc, records the first line of the next file | Records lines from the proc's own file |
| Other code over several lines: statements that a macro expands to (`LAZYADD`, `DB_VIEW`), conditions with several operators continued with `\`, list entries assigned over several lines, ternaries in text | 184, 217, 130 | Records further lines that it has read ahead to, also blank lines in a macro's expansion | Records the lines described above |

These differences can move breakpoints after a reload. When DreamMaker records a line it read
ahead to, its bytecode can stop on a statement sooner.

## Warnings

`CompiledProc::warnings` reports two kinds of behavior: differences from DreamMaker that dmasm
makes on purpose, and important DreamMaker quirks that dmasm copies. Reload passes these
diagnostics on.

`CompiledProc::byond_bugs` has the warnings about bugs in DreamMaker builds. It leaves out the
details of dmasm's workarounds and the differences that matter only for compatibility. It
includes the bugs listed below, division in parameter defaults, safe-call objects, logical
assignment before 516.1673, indexed pointers from 516.1668, `*(&x)` before 516.1668, the copied
quirks, and `pick()` ending in a zero weight before 516.1689. `Snapshot::known_bugs` collects
them across a project for `auxtools check`.

Each warning is a `diagnostic::Diagnostic` with a source line and an ID. Each compatibility
difference has its own ID. Each BYOND bug has a `Byond` ID, and its `byond_bugs` entry has the
same ID as its warning.

The compiler warns about division in parameter defaults, safe-call objects, logical assignment
before 516.1673, indexed pointer assignment from 516.1668, `*(&x)` before 516.1668,
out-of-range bit operands, `pick()` ending in a zero weight before 516.1689, angles that fold to
other values with glibc 2.41, and colors that fold to other values on Windows.

dmasm also warns about two DreamMaker quirks that it copies:

- In 516.1648–516.1669 and 516.1671, a constant `isnan` or `isinf` test emits its result and then
  `GetFlag`, so it returns the flag of the test before. From 516.1670, a test inside another
  constant expression, such as `!isnan(1)`, folds with that expression. This also holds in
  516.1671.
- Before 516.1685, if you change a list element through `?.`, and the value uses the cache, the
  list is taken from the cache value that the value left.

After `a.b.f()`, DreamMaker keeps `a.b` in the cache. So a later `a.b.x` in the same flow uses
the old object, even if `f()` put another object in `a.b`. dmasm copies this without a warning:
the pattern is common, and it matters only when the call replaces the field. If a call can
replace a field, read the field into a local.

These bugs affect both DreamMaker builds and reloads (`compiler/src/compiler/known_bugs.rs`):

- `continue LABEL` to a labelled `do` ... `while` inside a counted `for` loop crashes the
  server.
- Nested list loops with a `try` between them: when the code catches an error in the inner loop,
  the outer loop uses the inner loop's iterator. The outer loop can then end early, loop forever
  or crash.
- `operator<<=` and `operator>>=`: the result is discarded. `operator++` and `operator--` as a
  statement: they are called without an argument, and the result is discarded. `operator**` on
  a type with `operator*`: `a ** 2` calls `operator*`.
- `operator<=>` from 516.1674: `list.Find()` and `list ^ x` call it. From 516.1689, they take 0
  as a match. `operator~=` and `operator~!` in 516.1674 to 516.1688: the result is discarded.
- `Uncross()` of an area from 516.1683: the result is ignored.
- Reading `*p[i]` from 516.1668, as for its assignment above.

Differences only in debug locations or in encoding don't warn. In the validation codebases,
most warnings are divisions in tgstation's parameter defaults. Older builds also show the quirk
of safe list assignment.

## Compiler compatibility rules

These rules change the generated code. Focused tests cover each of them.

- User const variables, local consts included, have slots among the globals. A const read on
  its own reads the slot. A larger constant expression folds in the value. Built-in constants
  such as `NORTH` are emitted directly.
- Static locals are scoped to their block. Their scope (`StaticScope::Proc`) is the proc's path
  and the index of the declaration among the declarations of that name in the proc, from 0.
- Static and const access through `x::a::b` doesn't evaluate the objects before b.
  `/type::proc()` selects that type's override. `:` lookup searches subtypes depth first. A call
  on an unknown object uses the display name of the first declaration.
- After the first `for` loop, label, `switch` or `pick()`, a variable declared without a value
  is set to null explicitly.
- `for` loops, ternary else branches and logical operands whose value is discarded clear what
  the compiler knows about the cache. A label keeps that knowledge only if one path leads to
  it. Switch cases that share a target count as one path. A fallthrough after `return` still
  counts.
- Assignment statements clear cached field chains. List-entry writes, most declarations and
  assignments compiled as values don't. Among declarations, `var/L[] = new()` does clear them.
  Calls outside the cache clear chains. Calls on an object, `..()` and `spawn` keep them.
  `x.y ||= v` resolves its target before it tests it.
- Logical assignments resolve fields and list entries once. If the value's code needs the
  cache, they save the cache around it. Static and const reads through `.` and `global.x` don't
  need a save, but `::` accesses do. If the value indexes a list, the target's key is saved
  too.
- `del(x)` sets a writable target to null before it deletes the object. `del src` ends the
  proc. `del(world)` only deletes.
- An empty argument compiles to `PushVal null`, and an explicit null to `GetVar null`. An
  associative key `null = x` becomes the text `"null"`.
- `var/L[n]` emits `n; EmptyList`. A declaration with several dimensions uses `new /list`. In
  `var/x = a in L`, L is the declaration's input clause. The statement `x = a in L` parses as
  `(x = a) in L`.
- In return values, arguments and parentheses, associative `+ * & | ^` expressions are grouped
  again before folding. Math builtins fold, except `min` and `max`. Reversed `clamp` bounds are
  swapped. `X * X` uses `PowSquare` for supported values, but not for interpolated strings and
  fractions.
- Binary bit operations clamp their operands to i32 and keep 24 bits. `~` converts through u32.
  Tests cover its negative and overflow results.
- Safe assignment emits null checks for each field, call and index in the chain. Value
  expressions use `SetCacheJmpIfNull`, and statement targets use `SetCachePopJmpIfNull`.
- `PickSwitch` always jumps, so the compiler keeps its cache knowledge for the first option.
- `spawn` without a block applies to the next statement. Its code keeps the targets of the
  loops around it. A labelled `break` leaves the block. Leaving a `try` block emits `Catch` or
  `TryJmp`.
- `for (var/datum/T/x)` loops over all datums of type T. With `in world`, it loops over atoms
  and checks their type, so it finds no datums. `for (var/list/T/x in L)` checks that x is a T.
- `for (var/T/k, v in L)` filters and checks the type of the key, as `for (var/T/x in L)` does.
- `input()` takes four arguments, with nulls for missing ones, and an `in` source that is
  either built in or on the stack. `findlasttext` uses 0 as the default start and 1 as the
  default end.
- Parameters with an absolute path, and `...`, don't get parameter slots.
- Prefabs use entries in the instance table, with init procs. Their variable assignments are
  ordered by string ID, and source locations come before computed values. The assembler finds
  the instance through `AssembleEnv::get_instance`.

## State carried between emitters

Two checks need the code from before constant folding. `names_other_file` checks whether a
folded `/type::var` default comes from another file. `unfolded_has` checks whether the value in
`x.y ||= v` uses the cache. Both compile the expression again with folding off, through
`Compiler::speculate`. `speculate` discards the trial code and restores every other field,
warnings included, whether or not the trial compiled.

The fields below pass information from one emitter to a later one. Code sets each field just
before the code that uses it. Each entry says what would go wrong if other code came between.

- `regroup_root` is set before a return value, an argument or an expression in parentheses. The
  next `emit_inner_expr` takes it. If the expression is a binary operation, `emit_inner_expr`
  passes it to `binary_ops::emit` as `regroup_chain`. Otherwise it drops it. If other code came
  between, that code would be regrouped instead.
- `place` is set before the place of an assignment other than `=`: in `emit_place`, in the
  statement form and in `emit_conditional`. The next `emit_inner_expr` takes it and then records
  no read-ahead line or compound-term line. The arms that compile a rewritten expression (`&(x)`,
  `*(p)`, `locate(X) in a in b`), and a place in parentheses, compile the inner expression
  without it, so that expression records its lines. The statement form removes the parentheses
  first. Only a place written over several lines could differ, and DreamMaker refused the two
  such places that were tried.
- `safe_place` is set by `emit_conditional` in the debugger's code. `emit_inner_expr` takes it,
  and gives it back just before `follow::emit_typed` runs on the place's follows, after the
  term. So no code compiled for the term sees it.
- `list_elem_type` is set by `emit_inner_expr` just before `follow::emit_typed`, which takes it
  first.
- `skip_compound` is set for `return "[x]"` after its read-ahead line. The `emit_inner_expr` of
  the return value takes it. Such a value is never `.`, so the field is always taken.
- `explicit_src` is set where a chain starts with `src`, as in `src.x` or `new src.a.b()`. The
  next instruction, the chain's first load, clears it. A call's arguments save and restore it,
  because the object's `src.` does not apply to them. An instruction emitted before the load
  would clear it, and the chain would then reuse what a bare `x` left in the cache.
- `line_pending` means that a default's line, or a text initializer's read-ahead line, was
  recorded. `read_on_pending` means that a read-ahead line was recorded. With either set, the
  next compound term records its own line. Any line record clears them, so they last until the
  next line, not the next expression. Without debug information, no line is recorded and they
  stay set, which changes nothing.
- `other_file` means that the last line record named another file, such as a default's file,
  or would have if the default hadn't folded. The next line record then names the proc's file
  again first. Without debug information, it stays set, which does no harm.
- When a multiplication is the left operand of another multiplication, `mul_lhs` suppresses
  `PowSquare` for the inner multiplication. Parentheses do not change this rule.
- `condition_chain` is set while an `if` or `while` condition that is a `&&` or `||` compiles.
  The first `&&` or `||`, which is the condition itself, takes it.

Other fields are set around one piece of code and cleared after it:

- `implicit_type` holds the declared type of the place that a value is assigned to. The first
  implicit `new` or `astype()` in the value takes that type, as in DreamMaker: in
  `var/T/x = f(new)`, `new` makes a T.
- `global_field` is set by field reads. It is cleared before the value of a `||=` statement and
  taken after it.
- `proc_macro_ok` is set for the proc argument of `call(x, __PROC__)`. Before 515.1644,
  DreamMaker allows `__PROC__` there, as it does everywhere.
- `no_fold` and `live_only` are set for the trial compiles.

`read_to` is reset at each statement.

## Debugger code

`Compiler` compiles a proc from source with `compile_proc`. It also compiles the code that you
enter in the debugger: a proc body with `compile_body_in`, or a single expression with the
`compile_expr_*` functions. The debugger uses `is_body` to choose between the two. An
expression is compiled as the body `return EXPR`, without debug lines. So expressions use the
same code path as procs, which the comparison with DreamMaker covers. Both forms run in the
debugger's evaluation stub and return `list(result, params...)`.

The debugger's code is compiled as a global proc, the stub. It is compiled against a tree of
BYOND's built-in types, which is parsed once, with `Compiler::debugger` set. When the caller has
a paused frame, `expr_scope` answers questions about that frame. The debugger's code differs
from a proc's code only in these places:

| Where | Construct | In the debugger's code |
| --- | --- | --- |
| `Compiler::proc_macros_known`, `term::emit` | `__TYPE__`, `__PROC__`, `..()`, `.()` | Refused: the stub is not the paused proc and has no parent |
| `Compiler::emit_global_by_name`, `procs::find_var` | A bare name | The scope looks for a local of the body, then a parameter (the frame's variables), then a variable of `src`, then a global. If none has the name, the error is "undefined var". A global is read through the world's list of globals, or by the ID that the scope gives. Without a scope, the name is a global |
| `procs::const_lookup` | `vars` | A name like any other: BYOND's types declare it at the root as a const with no value |
| `term::emit`, `procs::emit_bare_call` | `f()` | `src.f()` if the scope says that `src` has the proc, otherwise a global proc |
| `follow::emit_typed` | `x.f()` | Called by name (`CallName`) if BYOND's types have no such proc. The world's types are not known, and a call by name also finds verbs |
| `follow::commit_field_buffer` | `global.x` | The same as a bare global. `global.vars` is the scope's list |
| `term::emit`, `Compiler::emit_inner_expr` | `/type/proc/name`, `/type::name()` | Looked up in the world through the scope |
| `Compiler::emit_inner_expr`, `fold::static_const` | `/type::var`, `x::a::b` | Read when the code runs. Not folded, and not resolved from a tree |
| `builtin_procs::emit_code`, `builtin_procs::declared_world_type`, `statements::implicit_type_for` | `astype(x)`, `istype(x)`, `var/T/x = new` | A declared type that BYOND's types lack is used by its written name, and found when the code is assembled |
| `term::prefab_value` | `/type{var = value}` | Refused: the instance would stay in the world's tables |
| `binary_ops::emit`, `fold::binary_value` | A bit operand beyond 32 bits | Left for the world to convert at runtime |
| `assignment::emit_conditional` | `a?.b?.c \|\|= v`, `a?[i] &&= v` as a value | Compiled by the `Safe` arms of `follow::emit_typed`, which leave a place. In a proc, only a plain `a?.b.c` compiles as a value, and the other forms are refused |

`compiler/tests/compile_body.rs` compiles expressions on their own and as `return EXPR`, and
checks that the instructions are the same.

## Unsupported forms

Compilation returns an error for constructs that dmasm doesn't support. Examples are reads from
a savefile into a savefile (`S["a"] >> S["b"]`), `load_resource()` on its own, and switch cases
that are not constant. The caller must then keep the existing bytecode or refuse the reload.

The assembler also needs the resources, strings, procs and prefab instances that the code uses,
through its environment. The resources must already be in the world's `.rsc`. The hot reload
caller must add any new entries to the runtime tables.

## Parser dependency

`Cargo.toml` uses [auxtools/SpacemanDMM](https://github.com/auxtools/SpacemanDMM) for parser and
preprocessor fixes that the compiler needs: separate alist terms, empty arguments, type clauses
in `for ... in`, macro stringification, source-location macros, raw strings, declaration flags,
pragma handling and metadata for absolute parameters.
