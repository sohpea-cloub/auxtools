mod compare;
mod dmb;
mod env;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

type Builds = (u32, u32);

struct Case {
    name: String,
    guards: String,
    source: String,
    builds: Option<Builds>,
    known: Vec<(Option<Builds>, String, String)>,
}

fn usage() -> ExitCode {
    eprintln!("usage: compare_dreammaker DREAMMAKER [CORPUS] [--out DIR] [--case NAME] [--show]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut positional = vec![];
    let mut out = PathBuf::from("target/compare-dreammaker");
    let mut filter = None;
    let mut show = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => match args.next() {
                Some(dir) => out = dir.into(),
                None => return usage(),
            },
            "--case" => filter = args.next(),
            "--show" => show = true,
            _ if a.starts_with('-') => return usage(),
            _ => positional.push(PathBuf::from(a)),
        }
    }
    let (dreammaker, corpus) = match positional.as_slice() {
        [dm] => (
            dm.clone(),
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/dreammaker"),
        ),
        [dm, corpus] => (dm.clone(), corpus.clone()),
        _ => return usage(),
    };
    let started = std::time::Instant::now();
    let dreammaker = std::fs::canonicalize(&dreammaker).unwrap_or(dreammaker);

    let byond = match dreammaker_build(&dreammaker) {
        Ok((major, build)) => dmasm_compiler::ByondVersion::new(major, build),
        Err(e) => {
            eprintln!("error: can't run DreamMaker: {e}.");
            return ExitCode::from(2);
        }
    };
    let cases = match read_corpus(&corpus) {
        Ok(cases) => cases,
        Err(e) => {
            eprintln!("error: can't read the corpus {}: {e}.", corpus.display());
            return ExitCode::from(2);
        }
    };
    let on =
        |builds: Option<Builds>| builds.is_none_or(|(from, to)| (from..=to).contains(&byond.build));
    let (cases, skipped): (Vec<_>, Vec<_>) = cases
        .into_iter()
        .filter(|c| filter.as_ref().is_none_or(|f| c.name.contains(f.as_str())))
        .partition(|c| on(c.builds));
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("error: can't create {}: {e}.", out.display());
        return ExitCode::from(2);
    }

    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()) * 4;
    let next = AtomicUsize::new(0);
    let results = Mutex::new(vec![]);
    std::thread::scope(|s| {
        for _ in 0..threads.min(cases.len()) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(case) = cases.get(i) else { break };
                let known = case
                    .known
                    .iter()
                    .filter(|(builds, ..)| on(*builds))
                    .map(|(_, path, why)| (path.clone(), why.clone()))
                    .collect();
                let outcome = run_case(case, &known, &dreammaker, &out, byond, show);
                results.lock().unwrap().push((i, outcome));
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|r| r.0);

    let mut total = compare::Outcome::default();
    let mut failed = 0;
    for (i, outcome) in &results {
        let case = &cases[*i];
        if outcome.findings.is_empty() {
            println!(
                "ok      {}: {} procs, {} exact, {} equivalent, {} known",
                case.name, outcome.procs, outcome.exact, outcome.equivalent, outcome.known
            );
        } else {
            failed += 1;
            println!("FAILED  {} ({})", case.name, case.guards);
            for f in &outcome.findings {
                println!("        {f}");
            }
        }
        total.procs += outcome.procs;
        total.exact += outcome.exact;
        total.equivalent += outcome.equivalent;
        total.known += outcome.known;
    }
    println!(
        "{} cases on {byond} ({} for other builds), {} procs: {} exact, {} equivalent, {} known differences; {failed} cases \
         failed, in {:.1}s",
        cases.len(),
        skipped.len(),
        total.procs,
        total.exact,
        total.equivalent,
        total.known,
        started.elapsed().as_secs_f32()
    );
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn dreammaker_command(dreammaker: &Path) -> Command {
    let mut command = Command::new(dreammaker);
    if let Some(dir) = dreammaker
        .parent()
        .filter(|d| d.join("libbyond.so").exists())
    {
        let mut paths = vec![dir.to_owned()];
        paths.extend(
            std::env::var_os("LD_LIBRARY_PATH")
                .iter()
                .flat_map(std::env::split_paths),
        );
        command.env(
            "LD_LIBRARY_PATH",
            std::env::join_paths(paths).unwrap_or_default(),
        );
    }
    command
}

fn dreammaker_build(dreammaker: &Path) -> Result<(u32, u32), String> {
    let output = dreammaker_command(dreammaker)
        .output()
        .map_err(|e| format!("{}: {e}", dreammaker.display()))?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    let version = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("DM compiler version "))
        .ok_or_else(|| format!("it didn't say its version: {}", text.trim()))?;
    parse_build(version).ok_or_else(|| format!("version {version}"))
}

fn parse_build(text: &str) -> Option<(u32, u32)> {
    let (major, build) = text.trim().split_once('.')?;
    Some((major.parse().ok()?, build.parse().ok()?))
}

fn parse_builds(text: &str) -> Option<Builds> {
    let (from, to) = text.trim().split_once('-')?;
    let bound = |t: &str, default: u32| match t.trim() {
        "" => Some(default),
        t => parse_build(t).map(|(_, build)| build),
    };
    bound(from, 0).zip(bound(to, u32::MAX))
}

fn read_corpus(dir: &Path) -> Result<Vec<Case>, String> {
    let mut files = vec![];
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|e| e == "dm") {
            files.push(path);
        }
    }
    files.sort();
    let mut cases = vec![];
    for path in files {
        let file = path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{file}: {e}"))?;
        let mut file_cases: Vec<Case> = vec![];
        for line in text.lines() {
            if let Some(guards) = line.strip_prefix("// Guards:") {
                file_cases.push(Case {
                    name: String::new(),
                    guards: guards.trim().to_owned(),
                    source: String::new(),
                    builds: None,
                    known: vec![],
                });
            }
            let Some(case) = file_cases.last_mut() else {
                return Err(format!(
                    "{file}: it doesn't start with a `// Guards:` line saying what it is for"
                ));
            };
            case.source.push_str(line);
            case.source.push('\n');
            let Some(comment) = line.strip_prefix("//").map(str::trim) else {
                continue;
            };
            if let Some(range) = comment.strip_prefix("Builds:") {
                let range = range.split_whitespace().next().unwrap_or("");
                case.builds = Some(
                    parse_builds(range)
                        .ok_or_else(|| format!("{file}: `Builds: {range}`, not A-B"))?,
                );
            } else if let Some(known) = comment.strip_prefix("Known") {
                let bad = || format!("{file}: `{line}`, not `// Known [A-B]: PATH  WHY`");
                let (builds, rest) = known.split_once(':').ok_or_else(bad)?;
                let builds = match builds.trim() {
                    "" => None,
                    range => Some(parse_builds(range).ok_or_else(bad)?),
                };
                let (path, why) = rest
                    .trim()
                    .split_once(char::is_whitespace)
                    .ok_or_else(bad)?;
                case.known
                    .push((builds, path.to_owned(), why.trim().to_owned()));
            }
        }
        let several = file_cases.len() > 1;
        for (i, mut case) in file_cases.into_iter().enumerate() {
            case.name = if several {
                format!("{file}-{}", i + 1)
            } else {
                file.clone()
            };
            cases.push(case);
        }
    }
    Ok(cases)
}

fn run_case(
    case: &Case,
    known: &HashMap<String, String>,
    dreammaker: &Path,
    out: &Path,
    byond: dmasm_compiler::ByondVersion,
    show: bool,
) -> compare::Outcome {
    let failed = |why: String| compare::Outcome {
        findings: vec![why],
        ..Default::default()
    };
    let dir = out.join(&case.name);
    let _ = std::fs::remove_dir_all(&dir);
    let written = std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(dir.join("case.dm"), &case.source))
        .and_then(|_| std::fs::write(dir.join("world.dme"), "#include \"case.dm\"\n"));
    if let Err(e) = written {
        return failed(format!("can't write it to {}: {e}", dir.display()));
    }
    let built = dreammaker_command(dreammaker)
        .arg("world.dme")
        .current_dir(&dir)
        .output();
    let dmb_path = dir.join("world.dmb");
    match built {
        Ok(o) if o.status.success() && dmb_path.exists() => {}
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            let errors: Vec<_> = text.lines().filter(|l| l.contains("error")).collect();
            return failed(format!("DreamMaker didn't build it: {}", errors.join("; ")));
        }
        Err(e) => return failed(format!("can't run DreamMaker: {e}")),
    }
    let data = match std::fs::read(&dmb_path) {
        Ok(d) => d,
        Err(e) => return failed(format!("can't read {}: {e}", dmb_path.display())),
    };
    let mut dmb = match dmb::Dmb::load(&data) {
        Ok(d) => d,
        Err(e) => return failed(format!("can't read DreamMaker's .dmb: {e}")),
    };
    if let Ok(rsc) = std::fs::read(dir.join("world.rsc")) {
        dmb.read_rsc(&rsc);
    }
    let dme = dir.join("dmasm.dme");
    let prelude = format!(
        "#undef SPACEMAN_DMM\n#undef DM_VERSION\n#define DM_VERSION {}\n#undef DM_BUILD\n#define DM_BUILD {}\n#include \"case.dm\"\n",
        byond.major, byond.build
    );
    if let Err(e) = std::fs::write(&dme, prelude) {
        return failed(format!("can't write {}: {e}", dme.display()));
    }
    compare::compare(&dme, &dmb, byond, known, show)
}
