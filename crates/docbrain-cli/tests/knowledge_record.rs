// SPDX-License-Identifier: MIT
//! `docbrain record` end to end: the file written, what is staged, the README and the pointer on first use, the
//! body from a flag, standard input or the editor, and what an interrupt or a failing git leaves behind.

#[path = "common/knowledge_repo.rs"]
mod knowledge_repo;

use std::fs::File;
use std::os::fd::FromRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use docbrain_knowledge::record::{Id, Parsed, parse_record};
use knowledge_repo::{Out, Repo};
use serde_json::Value;

const README: &str = include_str!("../src/knowledge/readme.md");
const HEADING: &str = "## Repository knowledge\n";
const CHARTS_REC: &str = "charts/.knowledge/records/7k2m9q4r8t1v-production-api-runs-at-least-two-replicas.md";
const ROOT_REC: &str = ".knowledge/records/9b1c4e7f2a6d-roll-out-the-api-one-zone-at-a-time.md";
const QUESTION: [&str; 5] = ["record", "question", "Who owns the docs site?", "--applies-to", "docs/**"];

/// The transcript's repository, committed on `main`: a values file, a runbook, a decision in `charts/` and a
/// procedure at the root for every file. Not a first use: the root has `.knowledge/`.
fn fixture() -> Repo {
    let r = Repo::new();
    r.write("charts/api/values.yaml", "replicaCount: 2\nimage:\n  tag: '1.3'\n")
        .write("docs/runbooks/api-rollout.md", "# Rollout\n")
        .write(
            CHARTS_REC,
            "---\nknowledge: 1\nid: '7k2m9q4r8t1v'\nkind: 'decision'\n\
             title: 'Production API runs at least two replicas'\nchecks:\n\
             \x20 - config_key: 'api/values.yaml#replicaCount'\n    must: '>= 2'\n---\n\
             A single replica fell over.\n",
        )
        .write(
            ROOT_REC,
            "---\nknowledge: 1\nid: '9b1c4e7f2a6d'\nkind: 'procedure'\n\
             title: 'Roll out the API one zone at a time'\n---\nDrain one zone.\n",
        );
    r.commit("fixture");
    r
}

/// A repository with no `.knowledge` anywhere: the first use.
fn fresh() -> Repo {
    let r = Repo::new();
    r.write("docs/index.md", "# Docs\n").write("charts/api/values.yaml", "replicaCount: 2\n");
    r.commit("fresh");
    r
}

fn question(body: &str) -> Vec<&str> {
    [&QUESTION[..], &["--body", body]].concat()
}

/// The paths staged, in git's order.
fn staged(r: &Repo) -> Vec<String> {
    r.git(&["diff", "--cached", "--name-only", "-z"]).split('\0').filter(|p| !p.is_empty()).map(str::to_owned).collect()
}

/// The record file named on the run's "Wrote and staged" line.
fn written(out: &Out) -> String {
    out.stdout
        .lines()
        .find_map(|l| l.strip_prefix("Wrote and staged ").filter(|p| p.contains("/records/")))
        .unwrap_or_else(|| panic!("no record written: {out:?}"))
        .to_owned()
}

fn read(r: &Repo, path: &str) -> String {
    std::fs::read_to_string(r.dir().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Every file under the work tree that git does not track, and every staged change: what a run left behind.
fn left(r: &Repo) -> (String, Vec<String>) {
    (r.git(&["status", "--porcelain", "--untracked-files=all"]), staged(r))
}

/// An executable script at `name` under the repository's scratch home; its path.
fn script(r: &Repo, name: &str, text: &str) -> PathBuf {
    let p = r.home().join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, text).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

/// `p` quoted for `sh`, as a `GIT_EDITOR` value holds it.
fn sh_quoted(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', r"'\''"))
}

/// A pseudo-terminal: the controlling side, and the side the binary gets as its standard input, so it runs on a
/// terminal. `posix_openpt` is in the C library on every platform the CLI ships for.
fn pty() -> (File, File) {
    // SAFETY: plain libc calls on descriptors this function opens; each result is checked before it is used, and
    // each descriptor is owned by exactly one `File`.
    unsafe {
        let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        assert!(master >= 0, "posix_openpt");
        assert_eq!((libc::grantpt(master), libc::unlockpt(master)), (0, 0));
        let name = libc::ptsname(master);
        assert!(!name.is_null(), "ptsname");
        let secondary = libc::open(name, libc::O_RDWR | libc::O_NOCTTY);
        assert!(secondary >= 0, "open the secondary side");
        (File::from_raw_fd(master), File::from_raw_fd(secondary))
    }
}

/// The binary on a terminal, in a process group of its own: `typed` is what the user types; `during` runs while
/// it is running, with its process id.
fn run_tty(r: &Repo, env: &[(&str, &str)], typed: &[u8], args: &[&str], during: impl FnOnce(u32)) -> Out {
    use std::io::Write;
    let (mut master, secondary) = pty();
    master.write_all(typed).unwrap();
    let mut c = Command::new(knowledge_repo::binary());
    knowledge_repo::hermetic(&mut c, r.home());
    c.current_dir(r.dir()).args(args).envs(env.iter().copied()).process_group(0);
    // A terminal session starts with Ctrl-C and Ctrl-\ at their defaults. A job started with `&` by a shell without
    // job control (a floor run in the background) starts with both ignored, an ignored signal stays ignored through
    // every exec, and a shell cannot trap one it was started ignoring: no editor here could take Ctrl-C (#882). The
    // binary gets a terminal's defaults, however the tests were launched.
    // SAFETY: between fork and exec the closure calls only sigaction(2), which is async-signal-safe, with a zeroed
    // `sigaction` that names `SIG_DFL`; it allocates nothing.
    unsafe {
        c.pre_exec(|| {
            for signal in [libc::SIGINT, libc::SIGQUIT] {
                let mut default: libc::sigaction = std::mem::zeroed();
                default.sa_sigaction = libc::SIG_DFL;
                if libc::sigaction(signal, &default, std::ptr::null_mut()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    c.stdin(Stdio::from(secondary)).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = c.spawn().unwrap();
    during(child.id());
    let out = child.wait_with_output().unwrap();
    drop(master);
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

/// The status an editor in these tests exits with when Ctrl-C has not reached it within its 30 s bound (#882).
const MISSED_CTRL_C: u8 = 97;

/// A Ctrl-C that never reached the editor fails the test, named, within the editor's bound: never a hang (#882).
fn ctrl_c_reached_the_editor(out: &Out) {
    assert!(
        !out.stderr.contains(&format!("the editor exited with status {MISSED_CTRL_C}.")),
        "Ctrl-C never reached the editor: it ran out its 30 s bound. {out:?}"
    );
}

/// Waits up to 20 seconds for `p` to exist.
fn wait_for(p: &Path) {
    let start = Instant::now();
    while !p.exists() {
        assert!(start.elapsed() < Duration::from_secs(20), "{} never appeared", p.display());
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn writes_stages_and_prints_the_file() {
    let r = fixture();
    let out = r.run(&[
        "record",
        "decision",
        "The API image stays on 1.3",
        "--must",
        "charts/api/values.yaml#image.tag == 1.3",
        "--must-exist",
        "charts/api/values.yaml",
        "--body",
        "1.4 drops the health endpoint the probes call.",
    ]);
    assert_eq!((out.code, out.stderr.as_str()), (0, ""), "{out:?}");
    let file = written(&out);
    assert!(
        file.starts_with("charts/.knowledge/records/") && file.ends_with("-the-api-image-stays-on-1-3.md"),
        "{file}"
    );
    assert_eq!(staged(&r), [file.as_str()], "the record alone is staged");
    let text = &out.stdout;
    assert!(text.starts_with("Already about these files:\n"), "{text}");
    for part in [
        format!("  holds  decision  Production API runs at least two replicas  ({CHARTS_REC})\n"),
        format!("  holds  procedure  Roll out the API one zone at a time  ({ROOT_REC})\n"),
    ] {
        assert!(text.contains(&part), "{part}\n{text}");
    }
    assert!(
        text.ends_with(&format!(
            "Checked in the working tree:\n  must charts/api/values.yaml#image.tag == 1.3  holds  1.3\n  \
             must charts/api/values.yaml exist  holds\nWrote and staged {file}\n"
        )),
        "{text}"
    );
}

#[test]
fn record_stages_the_file_it_writes() {
    let r = fixture();
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 0, "{out:?}");
    let file = written(&out);
    assert_eq!(staged(&r), [file.as_str()]);
    let blob = r.git(&["cat-file", "blob", &format!(":0:{file}")]);
    assert_eq!(blob, read(&r, &file), "the index holds the bytes written");
}

#[test]
fn the_bytes_are_lf_without_bom() {
    let r = fixture();
    let out = r.run(&question("\u{feff}First line.\r\nSecond line.\r\n\r\nA second paragraph.\r\n"));
    assert_eq!(out.code, 0, "{out:?}");
    let bytes = std::fs::read(r.dir().join(written(&out))).unwrap();
    assert!(
        !bytes.contains(&b'\r') && !bytes.starts_with(&[0xef, 0xbb, 0xbf]),
        "{:?}",
        String::from_utf8_lossy(&bytes)
    );
    assert!(String::from_utf8(bytes).unwrap().ends_with("---\nFirst line.\nSecond line.\n\nA second paragraph.\n"));
}

#[test]
fn the_readme_is_created_once_at_the_root() {
    let r = fresh();
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 0, "{out:?}");
    let file = written(&out);
    assert_eq!(read(&r, ".knowledge/README.md"), README);
    assert!(README.contains("When a `records/` directory passes 1,000 files, use one\nsubdirectory per year"));
    assert_eq!(staged(&r), [".knowledge/README.md".to_owned(), file]);
    assert!(out.stdout.contains("Wrote and staged .knowledge/README.md\n"), "{out:?}");
    r.git(&["commit", "-q", "-m", "first"]);

    let again = r.run(&[
        "record",
        "fact",
        "The docs site builds from docs/",
        "--must-exist",
        "docs/index.md",
        "--body",
        "The site generator reads docs/.",
    ]);
    assert_eq!(again.code, 0, "{again:?}");
    assert_eq!(staged(&r), [written(&again)], "the README is not written again");
    assert!(!again.stdout.contains("README"), "{again:?}");
}

#[test]
fn the_readme_examples_are_valid_records_once_given_an_id() {
    let examples: Vec<String> = README
        .split("\n    ---\n    knowledge: 1\n")
        .skip(1)
        .map(|rest| {
            let lines: Vec<&str> = rest.lines().take_while(|l| l.starts_with("    ")).map(|l| &l[4..]).collect();
            format!("---\nknowledge: 1\n{}\n", lines.join("\n"))
        })
        .collect();
    assert_eq!(examples.len(), 3, "{examples:?}");
    for (n, ex) in examples.iter().enumerate() {
        let placeholder = format!("'0000000000a{}'", n + 1);
        assert!(ex.contains(&placeholder), "{ex}");
        assert!(!matches!(parse_record("", ex.as_bytes()), Parsed::Valid(_)), "a 0000 id is invalid: {ex}");
        let id = Id::mint().unwrap();
        let minted = ex.replace(&placeholder, &format!("'{}'", id.as_str()));
        assert!(matches!(parse_record("", minted.as_bytes()), Parsed::Valid(_)), "{minted}");
    }
}

#[test]
fn first_use_with_a_nested_folder_still_writes_the_root_readme() {
    let r = fresh();
    let out = r.run(&[
        "record",
        "decision",
        "Production API runs at least two replicas",
        "--must",
        "charts/api/values.yaml#replicaCount >= 2",
        "--folder",
        "charts",
        "--body",
        "One replica fell over.",
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    let file = written(&out);
    assert!(file.starts_with("charts/.knowledge/records/"), "{file}");
    assert_eq!(read(&r, ".knowledge/README.md"), README);
    assert_eq!(staged(&r), [".knowledge/README.md".to_owned(), file]);
}

#[test]
fn a_root_folder_without_a_readme_is_left_so() {
    let r = fixture();
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(!r.dir().join(".knowledge/README.md").exists());
    assert!(!out.stdout.contains(HEADING), "no pointer after the first use: {out:?}");
}

#[test]
fn an_untracked_readme_is_never_replaced() {
    let r = fresh();
    r.write(".knowledge/README.md", "Our own words.\n");
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(read(&r, ".knowledge/README.md"), "Our own words.\n");
    assert_eq!(staged(&r), [written(&out)], "the README is the user's to stage");
}

#[test]
fn unrelated_unstaged_changes_stay_unstaged() {
    let r = fixture();
    r.write("docs/runbooks/api-rollout.md", "# Rollout, edited\n").write("notes.txt", "mine\n");
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(staged(&r), [written(&out)]);
    assert_eq!(r.git(&["diff", "--name-only"]), "docs/runbooks/api-rollout.md\n");
    assert!(r.git(&["status", "--porcelain"]).contains("?? notes.txt\n"));
}

#[test]
fn no_terminal_prints_the_pointer_once() {
    let r = fresh();
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout.matches(HEADING).count(), 1, "{out:?}");
    assert!(out.stdout.contains("a new AGENTS.md"), "where the lines go: {out:?}");
    assert!(!r.dir().join("AGENTS.md").exists(), "printed, never written, without a terminal");
    r.git(&["commit", "-q", "-m", "first"]);
    let again = r.run(&question("Still nobody."));
    assert_eq!(again.code, 0, "{again:?}");
    assert!(!again.stdout.contains(HEADING), "{again:?}");
}

#[test]
fn on_a_terminal_first_use_asks_and_yes_appends() {
    let r = fresh();
    r.write("AGENTS.md", "# Agents\n").commit("agents");
    let out = run_tty(&r, &[], b"y\n", &question("Nobody has said yet."), |_| {});
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("AGENTS.md"), "{out:?}");
    assert_eq!(read(&r, "AGENTS.md").matches(HEADING).count(), 1);
    assert!(!staged(&r).contains(&"AGENTS.md".to_owned()), "the pointer is never staged");
}

#[test]
fn editor_supplies_the_body() {
    let r = fixture();
    let editor = script(
        &r,
        "my editor/write-body",
        "#!/bin/sh\n[ \"$1\" = --wait ] || exit 9\nprintf 'Written in the editor.\\n\\nMore.\\n' > \"$2\"\n",
    );
    let value = format!("{} --wait", sh_quoted(&editor));
    let out = run_tty(&r, &[("GIT_EDITOR", value.as_str())], b"", &QUESTION, |_| {});
    assert_eq!(out.code, 0, "{out:?}");
    assert!(read(&r, &written(&out)).ends_with("---\nWritten in the editor.\n\nMore.\n"));
}

#[test]
fn an_empty_editor_result_is_refused() {
    let r = fixture();
    let before = left(&r);
    let out = run_tty(&r, &[("GIT_EDITOR", ":")], b"", &QUESTION, |_| {});
    assert_eq!((out.code, out.stderr.as_str()), (2, "Not written: the body is empty; its first paragraph says why.\n"));
    assert_eq!(left(&r), before);
}

#[test]
fn a_failing_editor_writes_nothing() {
    let r = fixture();
    let before = left(&r);
    let out = run_tty(&r, &[("GIT_EDITOR", "false")], b"", &QUESTION, |_| {});
    assert_eq!((out.code, out.stderr.as_str()), (3, "Not written: the editor exited with status 1.\n"), "{out:?}");
    assert_eq!(left(&r), before);
}

#[test]
fn a_bad_flag_is_refused_before_the_editor_opens() {
    let r = fixture();
    let mark = r.home().join("editor-ran");
    let editor = script(&r, "mark", &format!("#!/bin/sh\ntouch {}\necho Why. > \"$1\"\n", sh_quoted(&mark)));
    let args = ["record", "decision", "Five replicas", "--must", "charts/api/values.yaml#replicaCount >= 5"];
    let out = run_tty(&r, &[("GIT_EDITOR", &editor.display().to_string())], b"", &args, |_| {});
    assert_eq!(out.code, 2, "{out:?}");
    assert!(out.stderr.starts_with("Not written: every check must hold in the working tree.\n"), "{out:?}");
    assert!(!mark.exists(), "the editor opened for a record that could not be written");
}

/// A plain editor name runs itself, as git runs it, so one that is not installed says it cannot start rather than
/// passing a shell's 127 off as the editor's status.
#[test]
fn an_editor_that_is_not_installed_cannot_start() {
    let r = fixture();
    let before = left(&r);
    let out = run_tty(&r, &[("GIT_EDITOR", "docbrain-no-such-editor")], b"", &QUESTION, |_| {});
    assert_eq!(out.code, 3, "{out:?}");
    assert!(out.stderr.starts_with("cannot start the editor docbrain-no-such-editor: "), "{out:?}");
    assert_eq!(left(&r), before);
}

#[test]
fn ctrl_c_in_the_editor_writes_nothing() {
    let r = fixture();
    let before = left(&r);
    let index = std::fs::read(r.dir().join(".git/index")).unwrap();
    let mark = r.home().join("editor-started");
    // The mark is written by the process Ctrl-C reaches, never by a child of a shell: a `/bin/sh` that sees its
    // child exit 0 after Ctrl-C goes on to its next line.
    let editor = script(
        &r,
        "sleeps",
        &format!(
            "#!/bin/sh\nexec perl -e 'open my $f, \">\", $ARGV[0] or die; close $f; sleep 30; exit {MISSED_CTRL_C}' {}\n",
            sh_quoted(&mark)
        ),
    );
    let out = run_tty(&r, &[("GIT_EDITOR", &editor.display().to_string())], b"", &QUESTION, |pid| {
        wait_for(&mark);
        // SAFETY: a signal to the process group the binary leads.
        assert_eq!(unsafe { libc::kill(-(pid as i32), libc::SIGINT) }, 0);
    });
    ctrl_c_reached_the_editor(&out);
    assert_eq!((out.code, out.stderr.as_str()), (3, "Not written: the editor was interrupted.\n"), "{out:?}");
    assert_eq!(left(&r), before, "nothing new in the work tree or the index");
    assert_eq!(std::fs::read(r.dir().join(".git/index")).unwrap(), index);
}

#[test]
fn an_editor_that_takes_ctrl_c_keeps_record_alive() {
    let r = fixture();
    let mark = r.home().join("editor-started");
    let text = format!(
        "#!/bin/sh\ntrap 'printf \"Written after Ctrl-C.\\n\" > \"$1\"; exit 0' INT\ntouch {}\n\
         i=0; while [ $i -lt 30 ]; do sleep 1; i=$((i+1)); done; exit {MISSED_CTRL_C}\n",
        sh_quoted(&mark)
    );
    let editor = script(&r, "takes-ctrl-c", &text);
    let out = run_tty(&r, &[("GIT_EDITOR", &editor.display().to_string())], b"", &QUESTION, |pid| {
        wait_for(&mark);
        // SAFETY: a signal to the process group the binary leads.
        assert_eq!(unsafe { libc::kill(-(pid as i32), libc::SIGINT) }, 0);
    });
    ctrl_c_reached_the_editor(&out);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(read(&r, &written(&out)).ends_with("---\nWritten after Ctrl-C.\n"));
}

#[test]
fn no_editor_named_says_how_to_go_on() {
    let r = fixture();
    let before = left(&r);
    let out = run_tty(&r, &[("TERM", "dumb")], b"", &QUESTION, |_| {});
    assert_eq!(out.code, 3, "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
    for part in ["core.editor", "--body"] {
        assert!(out.stderr.contains(part), "{part}: {out:?}");
    }
    assert_eq!(left(&r), before);
}

/// A reader that has gone (`| head -0`) ends the output quietly, the pointer's lines too; the record is written.
#[test]
fn a_closed_stdout_keeps_the_exit_and_says_nothing() {
    let r = fresh();
    let mut c = Command::new(knowledge_repo::binary());
    knowledge_repo::hermetic(&mut c, r.home());
    let args = question("Nobody has said yet.");
    let mut child = c
        .current_dir(r.dir())
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stderr).as_ref()), (Some(0), ""));
    assert_eq!(staged(&r).len(), 2, "the README and the record");
}

#[test]
fn no_terminal_and_no_body_exits_3() {
    let r = fixture();
    let before = left(&r);
    let out = r.run(&QUESTION);
    assert_eq!(out.code, 3, "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
    assert!(out.stderr.contains("pass --body"), "{out:?}");
    assert_eq!(left(&r), before);
}

#[test]
fn body_from_stdin() {
    let r = fixture();
    let out = r.run_with(r.dir(), &[], Some(b"Read from standard input.\n"), &question("-"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(read(&r, &written(&out)).ends_with("---\nRead from standard input.\n"));
}

/// A record git ignores is never read (the trees judged are git's): refused before anything is written, naming the
/// rule. A rule that re-includes the records is no ignore.
#[test]
fn an_ignored_record_is_refused_before_anything_is_written() {
    let r = fresh();
    r.write(".gitignore", "build/\n.knowledge/\n").commit("ignore");
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 3, "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
    assert!(
        out.stderr.contains("git ignores .knowledge/records/") && out.stderr.contains(".gitignore:2:.knowledge/"),
        "{out:?}"
    );
    assert!(!r.dir().join(".knowledge").exists(), "nothing written: {out:?}");
    assert_eq!(left(&r), (String::new(), Vec::new()));

    r.write(".gitignore", "*.md\n!**/.knowledge/**\n").commit("re-include");
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(staged(&r).len(), 2, "the README and the record: {out:?}");

    // A folder whose name starts with `:` is checked as that folder, not read as pathspec magic.
    r.write(":ops/run.sh", "#!/bin/sh\n").write(".gitignore", ":ops/.knowledge/\n").commit("ops");
    let ops = ["record", "procedure", "Run it", "--must-exist", ":ops/run.sh", "--folder", ":ops", "--body", "Run it."];
    let out = r.run(&ops);
    assert_eq!(out.code, 3, "{out:?}");
    assert!(out.stderr.contains("git ignores :ops/.knowledge/records/"), "{out:?}");
    assert!(!r.dir().join(":ops/.knowledge").exists(), "nothing written: {out:?}");
}

/// The README is for people new to the folder; a rule that ignores it never stops the record.
#[test]
fn an_ignored_readme_is_left_out_and_the_record_written() {
    let r = fresh();
    r.write(".gitignore", "README.md\n").commit("ignore");
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!((out.code, out.stderr.as_str()), (0, ""), "{out:?}");
    assert_eq!(staged(&r), [written(&out)]);
    assert!(!out.stdout.contains("README"), "{out:?}");
    assert!(!r.dir().join(".knowledge/README.md").exists());
}

/// A write is confined as every read is (`SEMANTICS.md` §Paths and globs): a directory on the way to the record
/// that is a symbolic link is never followed, so nothing lands outside the repository.
#[test]
fn a_symbolic_link_on_the_way_is_never_written_through() {
    let outside = |r: &Repo| {
        let o = r.home().join("outside");
        std::fs::create_dir_all(&o).unwrap();
        o
    };
    let entries = |o: &Path| std::fs::read_dir(o).unwrap().count();

    let r = fresh();
    let o = outside(&r);
    std::os::unix::fs::symlink(&o, r.dir().join(".knowledge")).unwrap();
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 3, "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
    assert!(out.stderr.contains(".knowledge is a symbolic link"), "{out:?}");
    assert_eq!(entries(&o), 0, "nothing written outside: {out:?}");
    assert_eq!(left(&r), ("?? .knowledge\n".to_owned(), Vec::new()));

    let r = fresh();
    r.write(".knowledge/notes.txt", "kept\n").commit("folder");
    let o = outside(&r);
    std::os::unix::fs::symlink(&o, r.dir().join(".knowledge/records")).unwrap();
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 3, "{out:?}");
    assert!(out.stderr.contains(".knowledge/records is a symbolic link"), "{out:?}");
    assert_eq!(entries(&o), 0, "nothing written outside: {out:?}");
}

#[test]
fn a_locked_index_says_written_not_staged() {
    let r = fresh();
    r.write(".git/index.lock", "");
    let out = r.run(&question("Nobody has said yet."));
    std::fs::remove_file(r.dir().join(".git/index.lock")).unwrap();
    assert_eq!(out.code, 3, "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
    let file = out
        .stderr
        .split_whitespace()
        .find(|w| w.starts_with(".knowledge/records/"))
        .unwrap_or_else(|| panic!("the record is named: {out:?}"))
        .to_owned();
    assert!(r.dir().join(&file).exists(), "{file}");
    for part in ["Wrote ", ".knowledge/README.md", "git add --sparse -- ", "not stage", "docbrain record again"] {
        assert!(out.stderr.contains(part), "{part}: {out:?}");
    }
    assert!(staged(&r).is_empty());
}

#[test]
fn a_file_where_the_records_directory_goes_is_named_not_retried() {
    let r = fresh();
    r.write(".knowledge/records", "stray\n");
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 3, "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
    assert!(out.stderr.contains("cannot create the directory .knowledge/records"), "{out:?}");
    assert!(!out.stderr.contains("run the command again"), "running again fails the same way: {out:?}");
    assert_eq!(left(&r), ("?? .knowledge/records\n".to_owned(), Vec::new()), "the README written for it is gone");
}

#[test]
fn a_filter_that_rewrites_the_record_is_reported() {
    let r = fixture();
    r.write(".gitattributes", "*.md filter=upper\n").commit("attributes");
    r.git(&["config", "filter.upper.clean", "tr a-z A-Z"]);
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 3, "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
    assert!(out.stderr.contains("filter"), "{out:?}");
    let file = out.stderr.split_whitespace().find(|w| w.contains("/records/")).unwrap().trim_end_matches(',');
    assert!(r.dir().join(file).exists(), "{out:?}");
}

/// Runs `advice`, a command the binary printed, with `sh` in `cwd`; panics unless it succeeds.
fn follow(r: &Repo, cwd: &Path, advice: &str) {
    let mut c = Command::new("sh");
    knowledge_repo::hermetic(&mut c, r.home());
    let ran = c.current_dir(cwd).args(["-c", advice]).output().unwrap();
    assert!(ran.status.success(), "{advice}: {}", String::from_utf8_lossy(&ran.stderr));
}

/// The text of `out`'s stderr after `from` up to the next `;` or the end of the sentence: the command it advises.
fn advice(out: &Out, from: &str) -> String {
    let rest = out.stderr.split(from).nth(1).unwrap_or_else(|| panic!("no '{from}': {out:?}"));
    rest.split(';').next().unwrap().trim_end().trim_end_matches('.').to_owned()
}

#[test]
fn the_advised_git_add_works_from_a_subdirectory() {
    let r = fresh();
    let sub = r.dir().join("charts/api");
    let args = ["record", "fact", "Two replicas", "--must", "values.yaml#replicaCount >= 2", "--body", "Why."];
    r.write(".git/index.lock", "");
    let out = r.run_with(&sub, &[], None, &args);
    std::fs::remove_file(r.dir().join(".git/index.lock")).unwrap();
    assert_eq!(out.code, 3, "{out:?}");
    follow(&r, &sub, &advice(&out, "Stage them with "));
    assert_eq!(staged(&r).len(), 2, "the record and the README: {:?}", staged(&r));

    // The filter's remedy, from the same place: exempt the records, then the advised add stores the bytes written.
    let r = fresh();
    r.write(".gitattributes", "*.md filter=upper\n").commit("attributes");
    r.git(&["config", "filter.upper.clean", "tr a-z A-Z"]);
    let sub = r.dir().join("charts/api");
    let out = r.run_with(&sub, &[], None, &args);
    assert_eq!(out.code, 3, "{out:?}");
    // The index is refreshed before the person acts (a `git status`, an editor's): the record is then clean by its
    // stat, and an add that only looks at the stat keeps the filtered bytes.
    let file = staged(&r).into_iter().find(|f| f.contains("/records/")).unwrap();
    let past = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    File::options().write(true).open(r.dir().join(&file)).unwrap().set_modified(past).unwrap();
    r.git(&["status", "--porcelain"]);
    r.write(".gitattributes", "*.md filter=upper\n**/.knowledge/** -filter\n");
    follow(&r, &sub, &advice(&out, "in .gitattributes, then "));
    assert_eq!(r.git(&["cat-file", "blob", &format!(":0:{file}")]), read(&r, &file));
}

#[test]
fn a_record_outside_the_sparse_checkout_is_staged() {
    let sparse = || {
        let r = fresh();
        r.write("sub/deep/v.yaml", "k: 1\n").commit("sub");
        r.git(&["sparse-checkout", "set", "--cone", "charts"]);
        assert!(!r.dir().join("sub").exists(), "outside the cone");
        r
    };
    let args =
        ["record", "fact", "Outside the cone", "--must", "sub/deep/v.yaml#k == 1", "--folder", "sub", "--body", "Why."];
    let r = sparse();
    let out = r.run(&args);
    assert_eq!(out.code, 0, "{out:?}");
    let file = written(&out);
    assert!(file.starts_with("sub/.knowledge/records/"), "{file}");
    assert_eq!(staged(&r), vec![".knowledge/README.md".to_owned(), file.clone()]);

    // Not staged for another reason: the advice still works there.
    let r = sparse();
    r.write(".git/index.lock", "");
    let out = r.run(&args);
    std::fs::remove_file(r.dir().join(".git/index.lock")).unwrap();
    assert_eq!(out.code, 3, "{out:?}");
    follow(&r, r.dir(), &advice(&out, "Stage them with "));
    assert_eq!(staged(&r).len(), 2, "{:?}", staged(&r));
}

#[test]
fn a_submodule_on_the_way_is_refused_before_anything_is_written() {
    let inner = Repo::new();
    inner.write("x.txt", "x\n").commit("inner");
    let url = inner.dir().to_str().unwrap();
    for (at, args) in [
        (
            "sub2",
            vec![
                "record",
                "fact",
                "Into the submodule",
                "--folder",
                "sub2",
                "--applies-to",
                "sub2/**",
                "--body",
                "Why.",
            ],
        ),
        (".knowledge", vec!["record", "fact", "Shared records", "--applies-to", "docs/**", "--body", "Why."]),
    ] {
        let r = fresh();
        r.git(&["-c", "protocol.file.allow=always", "submodule", "add", "-q", url, at]);
        r.commit("submodule");
        let out = r.run(&args);
        assert_eq!(out.code, 2, "{at}: {out:?}");
        assert!(out.stderr.contains(&format!("{at} is a submodule")), "{at}: {out:?}");
        assert_eq!(left(&r), (String::new(), Vec::new()), "{at}: nothing written here");
        assert_eq!(
            r.git_in(&r.dir().join(at), &["status", "--porcelain", "--untracked-files=all"]),
            "",
            "{at}: nor there"
        );
    }
}

/// A file where the records folder goes is not a submodule: the write says what stopped it.
#[test]
fn a_file_where_the_records_go_is_named_by_the_write() {
    let r = fresh();
    r.write(".knowledge", "x\n").commit("a file named .knowledge");
    let out = r.run(&["record", "fact", "Shared records", "--applies-to", "docs/**", "--body", "Why."]);
    assert_eq!(out.code, 3, "{out:?}");
    assert!(out.stderr.contains("cannot create the directory .knowledge/records: "), "{out:?}");
    assert!(!out.stderr.contains("submodule"), "{out:?}");
}

#[test]
fn another_repository_in_the_work_tree_is_refused_before_anything_is_written() {
    for (at, args) in [
        (
            "vendor/x",
            vec![
                "record",
                "fact",
                "Into the nested one",
                "--folder",
                "vendor/x",
                "--applies-to",
                "vendor/x/**",
                "--body",
                "Why.",
            ],
        ),
        (".knowledge", vec!["record", "fact", "Shared records", "--applies-to", "docs/**", "--body", "Why."]),
    ] {
        let r = fresh();
        r.write(&format!("{at}/y.txt"), "y\n");
        r.git_in(&r.dir().join(at), &["init", "-q", "-b", "main"]);
        let nested = || r.git_in(&r.dir().join(at), &["status", "--porcelain", "--untracked-files=all"]);
        let (here, there) = (left(&r), nested());
        let out = r.run(&args);
        assert_eq!(out.code, 3, "{at}: {out:?}");
        assert!(out.stderr.contains(&format!("{at} holds another git repository")), "{at}: {out:?}");
        assert_eq!((left(&r), nested()), (here, there), "{at}: nothing written in either");
    }
}

#[test]
fn a_git_that_does_not_know_sparse_still_stages() {
    let r = fresh();
    let found = Command::new("sh").args(["-c", "command -v git"]).output().unwrap();
    let real = String::from_utf8(found.stdout).unwrap().trim().to_owned();
    let old = script(
        &r,
        "old-git/git",
        &format!(
            "#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = --sparse ] && {{ echo \"error: unknown option \\`sparse'\" >&2; exit 129; }}; done\n\
             exec {} \"$@\"\n",
            sh_quoted(Path::new(&real))
        ),
    );
    let path = format!("{}:{}", old.parent().unwrap().display(), std::env::var("PATH").unwrap());
    let env = [("PATH", path.as_str())];
    let args =
        ["record", "fact", "Two replicas", "--must", "charts/api/values.yaml#replicaCount >= 2", "--body", "Why."];
    let out = r.run_with(r.dir(), &env, None, &args);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(staged(&r).len(), 2, "{:?}", staged(&r));

    let r = fresh();
    r.write(".git/index.lock", "");
    let out = r.run_with(r.dir(), &env, None, &args);
    std::fs::remove_file(r.dir().join(".git/index.lock")).unwrap();
    assert_eq!(out.code, 3, "{out:?}");
    let advised = advice(&out, "Stage them with ");
    assert!(!advised.contains("--sparse"), "this git does not know it: {advised}");
    follow(&r, r.dir(), &advised);
    assert_eq!(staged(&r).len(), 2, "{:?}", staged(&r));
}

#[test]
fn a_folder_name_that_looks_like_pathspec_magic_is_staged() {
    let r = Repo::new();
    r.write(":ops/run.sh", "#!/bin/sh\n").commit("ops");
    let out = r.run(&[
        "record",
        "procedure",
        "Run the ops script",
        "--must-exist",
        ":ops/run.sh",
        "--folder",
        ":ops",
        "--body",
        "It sets up the host.",
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    let file = written(&out);
    assert!(file.starts_with(":ops/.knowledge/records/"), "{file}");
    assert!(staged(&r).contains(&file), "{:?}", staged(&r));
}

#[test]
fn the_file_mode_is_what_an_editor_writes() {
    let r = fixture();
    let out = r.run(&question("Nobody has said yet."));
    assert_eq!(out.code, 0, "{out:?}");
    let file = r.dir().join(written(&out));
    let beside = file.with_file_name("beside");
    File::create(&beside).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&file), mode(&beside));
}

#[test]
fn the_written_record_holds_in_status() {
    let r = fixture();
    let out = r.run(&[
        "record",
        "decision",
        "The API image stays on 1.3",
        "--must",
        "charts/api/values.yaml#image.tag == 1.3",
        "--body",
        "1.4 drops the health endpoint.",
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    let file = written(&out);
    let all: Value = serde_json::from_str(&r.run(&["status", "--json"]).stdout).unwrap();
    let rec = all["records"].as_array().unwrap().iter().find(|x| x["record_path"] == file.as_str()).unwrap();
    assert_eq!(rec["state"], "holds", "{rec}");

    let gate = r.run(&["status", "--base", "main", "--json"]);
    let g: Value = serde_json::from_str(&gate.stdout).unwrap_or_else(|e| panic!("{e}: {gate:?}"));
    assert_eq!(g["report"]["knowledge"]["added"], serde_json::json!([file.clone()]), "{g}");
    let ctx = r.run(&["context", "--json", "--base", "main", "charts/api/values.yaml"]);
    let c: Value = serde_json::from_str(&ctx.stdout).unwrap_or_else(|e| panic!("{e}: {ctx:?}"));
    let rec = c["records"].as_array().unwrap().iter().find(|x| x["record_path"] == file.as_str()).unwrap();
    assert_eq!(rec["review"], "not-reviewed", "{rec}");
}

#[test]
fn outside_a_work_tree_exits_3() {
    let r = fixture();
    let elsewhere = tempfile::tempdir().unwrap();
    let out = r.run_with(elsewhere.path(), &[], None, &question("Nobody has said yet."));
    assert_eq!(out.code, 3, "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0, "nothing written");
}

/// A record refused after the editor closes keeps what was typed, and says where and how to pass it back.
#[test]
fn the_body_typed_in_the_editor_is_kept_when_the_record_is_refused() {
    let r = fixture();
    let before = left(&r);
    let body = "Rotated the key AKIAIOSFODNN7EXAMPLE last week.\n";
    let src = r.home().join("typed.md");
    std::fs::write(&src, body).unwrap();
    let editor = script(&r, "secret", &format!("#!/bin/sh\ncp {} \"$1\"\n", sh_quoted(&src)));
    let out = run_tty(&r, &[("GIT_EDITOR", &editor.display().to_string())], b"", &QUESTION, |_| {});
    assert_eq!(out.code, 2, "{out:?}");
    assert!(out.stderr.starts_with("Not written: a secret-shaped value"), "{out:?}");
    let word = out
        .stderr
        .lines()
        .find_map(|l| l.strip_prefix("The body you wrote is kept in "))
        .and_then(|l| l.split_once("; pass it back with --body - < "))
        .map(|(a, b)| {
            assert_eq!(format!("{a}."), b, "one path, named twice");
            a.to_owned()
        })
        .unwrap_or_else(|| panic!("{out:?}"));
    let kept = Command::new("sh").arg("-c").arg(format!("cat < {word}")).output().unwrap();
    assert_eq!(String::from_utf8(kept.stdout).unwrap(), body, "the advice reads back what was typed");
    Command::new("sh").arg("-c").arg(format!("rm -- {word}")).status().unwrap();
    assert_eq!(left(&r), before);
}

#[test]
fn a_record_git_would_ignore_is_refused_before_the_editor_opens() {
    let r = fresh();
    r.write(".gitignore", ".knowledge/\n").commit("ignore");
    let mark = r.home().join("editor-ran");
    let editor = script(&r, "mark", &format!("#!/bin/sh\ntouch {}\necho Why. > \"$1\"\n", sh_quoted(&mark)));
    let before = left(&r);
    let out = run_tty(&r, &[("GIT_EDITOR", &editor.display().to_string())], b"", &QUESTION, |_| {});
    assert_eq!(out.code, 3, "{out:?}");
    assert!(out.stderr.starts_with("Not written: git ignores "), "{out:?}");
    assert!(!mark.exists(), "the editor opened for a record that could not be written");
    assert_eq!(left(&r), before);
}

/// `core.safecrlf=true` with `core.autocrlf=true` makes `git add` refuse a file with LF line endings. The
/// working-tree view and the record's own stage are not the user's commit: both go on, and the bytes staged are the
/// record's.
#[test]
fn a_repository_that_refuses_irreversible_line_endings_still_reads_and_records() {
    let r = fixture();
    r.git(&["config", "core.autocrlf", "true"]);
    r.git(&["config", "core.safecrlf", "true"]);
    r.write("charts/api/values.yaml", "replicaCount: 3\nimage:\n  tag: '1.3'\n");
    let out = r.run(&["context", "charts/api/values.yaml"]);
    assert_eq!(out.code, 0, "{out:?}");
    let out = r.run(&[
        "record",
        "decision",
        "Three replicas",
        "--must",
        "charts/api/values.yaml#replicaCount == 3",
        "--body",
        "Why.",
    ]);
    assert_eq!((out.code, out.stderr.as_str()), (0, ""), "{out:?}");
    let file = written(&out);
    assert_eq!(staged(&r), [file.as_str()]);
    assert_eq!(r.git(&["cat-file", "blob", &format!(":0:{file}")]), read(&r, &file));
}

/// A directory named in another case than the working tree's is the same directory on a case-insensitive file
/// system, and a second one beside it elsewhere: refused before anything is written, on every file system.
#[test]
fn a_path_spelled_in_another_case_is_refused_before_anything_is_written() {
    let r = fresh();
    let before = left(&r);
    for (args, at, twin) in [
        (&["--folder", "Charts", "--applies-to", "Charts/**"][..], "Charts", "charts"),
        (&["--applies-to", "Charts/**"][..], "Charts", "charts"),
        (&["--applies-to", "charts/API/*.yaml"][..], "charts/API", "charts/api"),
    ] {
        let out = r.run(&[&["record", "fact", "Case folder", "--body", "Why."][..], args].concat());
        assert_eq!(
            (out.code, out.stderr.as_str()),
            (
                2,
                format!("Not written: {at} differs from {twin} only in case; spell it as the working tree does.\n")
                    .as_str()
            ),
            "{args:?}"
        );
        assert_eq!(left(&r), before, "{args:?}");
    }
}

/// `git add` that ends without the record in the index (here a hook puts the index back) is reported as a record
/// not staged, in git's words, never blamed on a filter.
#[test]
fn a_record_left_out_of_the_index_is_reported_as_not_staged() {
    let r = fresh();
    script(&r, "hooks/post-index-change", "#!/bin/sh\n[ -n \"$IN_HOOK\" ] && exit 0\nIN_HOOK=1 git read-tree HEAD\n");
    r.git(&["config", "core.hooksPath", &r.home().join("hooks").display().to_string()]);
    let out = r.run(&["record", "fact", "Left out", "--applies-to", "docs/**", "--body", "Why."]);
    assert_eq!(out.code, 3, "{out:?}");
    let file = out
        .stderr
        .strip_prefix("Wrote ")
        .and_then(|l| l.split_once(" but did not stage it: "))
        .map(|(f, _)| f.to_owned())
        .unwrap_or_else(|| panic!("{out:?}"));
    assert!(file.starts_with(".knowledge/records/"), "{out:?}");
    assert!(out.stderr.contains("not in the index"), "{out:?}");
    assert!(!out.stderr.contains("filter"), "{out:?}");
    assert!(r.dir().join(file).exists());
}

/// Without --body and without a terminal there is no one to type a body: refused at once, the editor never started.
#[test]
fn no_body_without_a_terminal_is_refused_before_any_editor() {
    let r = fixture();
    let mark = r.home().join("editor-ran");
    let editor = script(&r, "mark", &format!("#!/bin/sh\ntouch {}\necho Why. > \"$1\"\n", sh_quoted(&mark)));
    let before = left(&r);
    let out = r.run_with(r.dir(), &[("GIT_EDITOR", &editor.display().to_string())], None, &QUESTION);
    assert_eq!(
        (out.code, out.stderr.as_str()),
        (
            3,
            "No body: pass --body <TEXT>, or --body - to read it from standard input; the editor opens only on a \
             terminal.\n"
        )
    );
    assert!(!mark.exists(), "an editor started with no one to type in it");
    assert_eq!(left(&r), before);
}

/// The root README is written before the record: a root that cannot take it stops the record before it is written.
#[test]
fn a_root_readme_that_cannot_be_written_stops_the_record_before_it_is_written() {
    // SAFETY: geteuid has no failure mode and reads only this process's effective user id.
    if unsafe { libc::geteuid() } == 0 {
        return; // permissions do not bind root
    }
    let r = fresh();
    let before = left(&r);
    std::fs::set_permissions(r.dir(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let out = r.run(&[
        "record",
        "fact",
        "Two replicas",
        "--must",
        "charts/api/values.yaml#replicaCount >= 2",
        "--folder",
        "charts",
        "--body",
        "Why.",
    ]);
    std::fs::set_permissions(r.dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out.code, 3, "{out:?}");
    assert!(out.stderr.starts_with("Not written: cannot write .knowledge/README.md: "), "{out:?}");
    assert_eq!(left(&r), before);
}

/// Every file a check names is looked up for the records already about it, not only the first.
#[test]
fn records_already_about_each_named_file_are_listed() {
    let r = fixture();
    r.write(
        "docs/.knowledge/records/3c5d7e9f1a2b-docs-are-built-nightly.md",
        "---\nknowledge: 1\nid: '3c5d7e9f1a2b'\nkind: 'fact'\ntitle: 'Docs are built nightly'\n\
         applies_to: ['**']\n---\nThe site job runs at night.\n",
    )
    .commit("docs record");
    let out = r.run(&[
        "record",
        "decision",
        "The API image stays on 1.3",
        "--must",
        "charts/api/values.yaml#image.tag == 1.3",
        "--must-exist",
        "docs/runbooks/api-rollout.md",
        "--body",
        "Why.",
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    let listing = out.stdout.split("Checked in the working tree:").next().unwrap();
    for title in ["Production API runs at least two replicas", "Docs are built nightly"] {
        assert!(listing.contains(title), "{title}: {out:?}");
    }
}

/// A glob names no file: nothing is listed as already about it.
#[test]
fn a_glob_check_lists_nothing_as_already_about_it() {
    let r = fixture();
    let out = r.run(&["record", "fact", "Runbooks exist", "--must-exist", "docs/runbooks/*.md", "--body", "Why."]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(!out.stdout.contains("Already about these files:"), "{out:?}");
}

/// A folder git reads as pathspec magic (a leading `:`) is advised literally, and the advice stages it.
#[test]
fn the_advised_git_add_works_for_a_folder_git_reads_as_magic() {
    let r = Repo::new();
    r.write(":ops/v.yaml", "k: 1\n").commit("ops");
    r.write(".git/index.lock", "");
    let out = r.run(&["record", "fact", "Ops", "--must", ":ops/v.yaml#k == 1", "--folder", ":ops", "--body", "Why."]);
    std::fs::remove_file(r.dir().join(".git/index.lock")).unwrap();
    assert_eq!(out.code, 3, "{out:?}");
    follow(&r, r.dir(), &advice(&out, "Stage them with "));
    assert_eq!(staged(&r).len(), 2, "the record and the README: {:?}", staged(&r));
}

/// A path of plain characters is advised as it is, unquoted: the command reads as a person would type it.
#[test]
fn the_advice_names_a_plain_path_bare() {
    let r = fresh();
    r.write(".git/index.lock", "");
    let out = r.run(&[
        "record",
        "fact",
        "Two replicas",
        "--must",
        "charts/api/values.yaml#replicaCount >= 2",
        "--folder",
        "charts",
        "--body",
        "Why.",
    ]);
    std::fs::remove_file(r.dir().join(".git/index.lock")).unwrap();
    assert_eq!(out.code, 3, "{out:?}");
    let written = out.stderr.strip_prefix("Wrote ").unwrap().split(" but did not stage ").next().unwrap();
    let names: Vec<&str> = written.split(" and ").collect();
    assert_eq!(names.len(), 2, "{out:?}");
    // `--sparse` is git's, from 2.35; the paths are the point here.
    assert_eq!(advice(&out, "Stage them with "), format!("git add --sparse -- {}", names.join(" ")));
}

/// #749: a value written in quotes is the text inside them, as a person writes a string; quoted in either kind or
/// bare, it is one check.
#[test]
fn a_quoted_value_is_the_text_inside_its_quotes() {
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace.package]\nrust-version = \"1.97\"\n").commit("manifest");
    let key = "Cargo.toml#workspace.package.rust-version";
    let out = r.run(&["record", "fact", "MSRV", "--must", &format!("{key} == '1.97'"), "--body", "Why."]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains(&format!("must {key} == 1.97  holds")), "{out:?}");
    for same in [format!("{key} == \"1.97\""), format!("{key} == 1.97")] {
        let out = r.run(&["record", "fact", "MSRV again", "--must", &same, "--body", "Why."]);
        assert_eq!(out.code, 2, "{same}: {out:?}");
        assert!(out.stderr.starts_with("Not written: the same check is in .knowledge/records/"), "{same}: {out:?}");
    }
    let out = r.run(&["status"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("holds"), "{out:?}");
}
