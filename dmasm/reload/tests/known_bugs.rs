use dmasm_compiler::diagnostic::Id;
use dmasm_compiler::ByondVersion;
use dmasm_reload::reload::{KnownBugs, Snapshot};

const CODE: &str = "\
/datum/num/proc/operator<<=(b)
\treturn src
/datum/num/proc/operator~=(b)
\treturn 1
/datum/num/proc/operator+(b)
\treturn src
/area/hall/Uncross(atom/movable/mover)
\treturn 0
/turf/floor/Uncross(atom/movable/mover)
\treturn 0
/proc/crash()
\tfor (var/i in 1 to 3)
\t\touter:
\t\t\tdo
\t\t\t\tif (i)
\t\t\t\t\tcontinue outer
\t\t\twhile (0)
/proc/caught(list/a, list/b)
\tfor (var/x in a)
\t\ttry
\t\t\tfor (var/y in b)
\t\t\t\tx += y
\t\tcatch
/proc/fine(list/a, list/b)
\tfor (var/x in a)
\t\tfor (var/y in b)
\t\t\ttry
\t\t\t\tx += y
\t\t\tcatch
/proc/pointers(list/L, p)
\tvar/a = *L[1]
\t*L[2] = a
\ta = max(1, -*L[3])
\ta = (*p)[1]
\treturn \"[*L[4]]\"
/datum/num/proc/weighted(a, b = a.len/10)
\treturn pick(1; a, 0; b)
/proc/refused()
\treturn &5
/datum/num/var/n
/datum/num/proc/me(x)
\treturn src
/proc/receiver(datum/num/a, datum/num/b)
\ta?.me().n = b.n
\ta?.me(b.n).n = 1
\ta?.me().n = 1
";

fn checked(byond: ByondVersion) -> KnownBugs {
    let ctx = dreammaker::Context::default();
    byond.read_as(&ctx);
    let pp = dreammaker::Preprocessor::from_buffer(&ctx, "code.dm".into(), CODE);
    let mut parser = dreammaker::Parser::new(&ctx, pp);
    parser.enable_procs();
    let tree = parser.parse_object_tree();
    assert!(ctx.errors().is_empty(), "{:?}", ctx.errors());
    let mut snapshot = Snapshot::from_tree(ctx, tree);
    snapshot.set_byond(byond);
    snapshot.known_bugs()
}

fn found(byond: ByondVersion) -> (Vec<(u32, String, String)>, Vec<String>) {
    let found = checked(byond);
    assert!(
        found.bugs.iter().all(|bug| bug.file == "code.dm"),
        "{:?}",
        found.bugs
    );
    assert!(
        found.bugs.iter().all(|bug| !bug.text.contains("dmasm")),
        "{:?}",
        found.bugs
    );
    let bugs = found
        .bugs
        .into_iter()
        .map(|bug| {
            let start = bug.text.split([':', ';']).next().unwrap().to_owned();
            (bug.line.unwrap(), bug.proc.to_string(), start)
        })
        .collect();
    let not_checked = found
        .not_checked
        .into_iter()
        .map(|(proc, file, error)| format!("{proc} ({file}): {error}"))
        .collect();
    (bugs, not_checked)
}

fn row(line: u32, proc: &str, start: &str) -> (u32, String, String) {
    (line, proc.to_owned(), start.to_owned())
}

const LOOPED: &str = "nested list loops separated by `try`";
const CONTINUED: &str =
    "`continue outer` to a labelled `do` ... `while` inside a counted `for` loop \
                         crashes the server";
const POINTER: &str = "`*p[i]`";
const RECEIVER: &str =
    "`?.` chains that continue after a call or list access (`a?.f(b.x).v = b.y`)";

#[test]
fn a_codebase_is_checked_by_file_and_line() {
    let (bugs, not_checked) = found(ByondVersion::new(516, 1688));
    assert_eq!(
        bugs,
        [
            row(1, "/datum/num/operator<<=", "`operator<<=`"),
            row(3, "/datum/num/operator~=", "`operator~=`"),
            row(7, "/area/hall/Uncross", "`Uncross()` of an area"),
            row(16, "/crash", CONTINUED),
            row(21, "/caught", LOOPED),
            row(31, "/pointers", POINTER),
            row(32, "/pointers", POINTER),
            row(33, "/pointers", POINTER),
            row(35, "/pointers", POINTER),
            row(
                36,
                "/datum/num/weighted",
                "parameter b's default divides a var by a number"
            ),
            row(
                37,
                "/datum/num/weighted",
                "pick() ends with a zero-weight entry, where DreamMaker's code jumps out of the proc"
            ),
            row(44, "/receiver", RECEIVER),
            row(45, "/receiver", RECEIVER),
        ]
    );
    assert_eq!(not_checked.len(), 1, "{:?}", not_checked);
    assert!(
        not_checked[0].starts_with("/refused (code.dm): "),
        "{:?}",
        not_checked
    );
}

#[test]
fn only_the_bugs_of_the_build_are_found() {
    let (bugs, _) = found(ByondVersion::new(516, 1689));
    assert_eq!(
        bugs,
        [
            row(1, "/datum/num/operator<<=", "`operator<<=`"),
            row(7, "/area/hall/Uncross", "`Uncross()` of an area"),
            row(16, "/crash", CONTINUED),
            row(21, "/caught", LOOPED),
            row(31, "/pointers", POINTER),
            row(32, "/pointers", POINTER),
            row(33, "/pointers", POINTER),
            row(35, "/pointers", POINTER),
            row(
                36,
                "/datum/num/weighted",
                "parameter b's default divides a var by a number"
            ),
            row(44, "/receiver", RECEIVER),
            row(45, "/receiver", RECEIVER),
        ]
    );
    let (bugs, _) = found(ByondVersion::new(515, 1647));
    assert_eq!(
        bugs,
        [
            row(1, "/datum/num/operator<<=", "`operator<<=`"),
            row(16, "/crash", CONTINUED),
            row(21, "/caught", LOOPED),
            row(
                36,
                "/datum/num/weighted",
                "parameter b's default divides a var by a number"
            ),
            row(
                37,
                "/datum/num/weighted",
                "pick() ends with a zero-weight entry, where DreamMaker's code jumps out of the proc"
            ),
            row(44, "/receiver", RECEIVER),
            row(45, "/receiver", RECEIVER),
        ]
    );
}

#[test]
fn findings_have_their_bugs_ids() {
    let ids: Vec<Id> = checked(ByondVersion::new(516, 1688))
        .bugs
        .iter()
        .map(|bug| bug.id)
        .collect();
    assert_eq!(
        ids,
        [
            Id::ByondShiftAssignDiscarded,
            Id::ByondEquivalenceDiscarded,
            Id::ByondAreaUncross,
            Id::ByondContinueToDoWhile,
            Id::ByondListLoopsAcrossTry,
            Id::ByondPointerIndexDropped,
            Id::ByondPointerIndexDropped,
            Id::ByondPointerIndexDropped,
            Id::ByondPointerIndexDropped,
            Id::ByondDefaultDivisionDropped,
            Id::ByondZeroWeightPick,
            Id::ByondSafeChainReceiver,
            Id::ByondSafeChainReceiver,
        ]
    );
}
