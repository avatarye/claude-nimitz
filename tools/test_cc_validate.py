#!/usr/bin/env python3
"""Tests of tools/cc-validate's own logic, with a fake shell: no git, cargo
or claude runs. `python3 tools/test_cc_validate.py`"""
import importlib.machinery
import importlib.util
import io
import json
import shutil
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
_loader = importlib.machinery.SourceFileLoader("cc_validate", str(HERE / "cc-validate"))
_spec = importlib.util.spec_from_loader("cc_validate", _loader)
ccv = importlib.util.module_from_spec(_spec)
_loader.exec_module(ccv)

RUN = "     Running unittests src/lib.rs (target/debug/deps/x)\n"
UNIT_OK = RUN + "test a::one ... ok\ntest result: ok. 640 passed; 0 failed; 0 ignored\n"
E2E_OK = RUN + "test e2e_00 ... ok\ntest result: ok. 27 passed; 0 failed; 0 ignored\n"
SHA = "43f2441" + "0" * 33


class FakeSh:
    """Answers commands by the first rule whose words all appear in it."""

    def __init__(self, rules):
        self.rules = rules  # [(substring, Res | callable(cmd, cwd) -> Res)]
        self.calls = []
        self.killed = set()

    def run(self, cmd, cwd=None, extra=None, timeout=600, log=None):
        line = " ".join(map(str, cmd))
        self.calls.append((line, extra or {}))
        for key, ans in self.rules:
            if key in line:
                return ans(cmd, cwd) if callable(ans) else ans
        return ccv.Res(0, "")

    def ran(self, key):
        return [c for c, _ in self.calls if key in c]


# a fleet with a squadron and a ship (the guard walks every member)
STATUS = {"fleets": [{"id": "HOME",
                      "admiral": {"id": "HOME", "role": "admiral", "session": {"session_id": "s1", "pid": 11}},
                      "squadrons": [{"id": "HOME/GH", "commodore": {"id": "HOME/GH", "role": "commodore", "session": {"session_id": "s2", "pid": 12}},
                                     "ships": [{"id": "HOME/GH/DEV", "role": "ship", "session": {"session_id": "s3", "pid": 13}},
                                               {"id": "HOME/GH/OFF", "role": "ship", "session": None}]}]}]}


def lay_repo(root, max_v="2.1.296"):
    files = {
        "docs/compat.toml": f'min = "2.1.288"\nmax = "{max_v}"\n',
        "README.md": f"# claude-nimitz\n\nStatus: in testing (Claude Code 2.1.288 - {max_v}).\n",
        "crates/nimitz-core/src/compat.rs": f'        assert_eq!(c.max, "{max_v}");\n',
        "crates/nimitz-core/src/check.rs": f'        snap.cc_version = Some("{ccv.next_patch(max_v)}".into());\n',
        "plugin/.claude-plugin/plugin.json": json.dumps({"name": "nimitz", "version": "0.4.2"}),
    }
    for rel, s in files.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(s)


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="ccv-test-"))
        self.home = self.tmp / "home"
        self.repo = self.tmp / "repo"
        lay_repo(self.repo)
        self.cfg = ccv.Cfg(repo=self.repo, home=self.home, cache=self.tmp / "cache", versions=self.tmp / "versions",
                           claude_home=self.home / ".claude", e2e_base=self.tmp / "e2e")
        (self.cfg.claude_home / "plugins").mkdir(parents=True)
        (self.cfg.claude_home / "plugins" / "installed_plugins.json").write_text(
            json.dumps({"plugins": {"nimitz@claude-nimitz": [{"version": "0.4.2"}]}}))

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def rules(self, version="2.1.297", porcelain="", branch="dev", counts="0\t0", unit=UNIT_OK, e2e=E2E_OK):
        status = STATUS

        def add_worktree(cmd, cwd):
            shutil.copytree(self.repo, Path(cmd[cmd.index("-b") + 2]))
            return ccv.Res(0, "")

        return [
            ("claude --version", ccv.Res(0, f"{version} (Claude Code)\n")),
            ("status --porcelain", ccv.Res(0, porcelain)),
            ("symbolic-ref", ccv.Res(0, branch + "\n")),
            ("rev-list --left-right", ccv.Res(0, counts + "\n")),
            ("nimitz status --json", ccv.Res(0, json.dumps(status))),
            ("worktree add", add_worktree),
            ("rev-parse --short", ccv.Res(0, "abc1234\n")),
            ("rev-parse dev", ccv.Res(0, SHA + "\n")),
            ("--test e2e", ccv.Res(0 if "FAILED" not in e2e else 101, e2e)),
            ("test --workspace", ccv.Res(0 if "FAILED" not in unit else 101, unit)),
            ("test -q -p nimitz-core", ccv.Res(0, "test result: ok. 300 passed\n")),
            ("nimitz doctor", ccv.Res(0, "[OK] pinned formats: 18 session/job file(s) carry every field nimitz reads\n[OK] doctor: ready\n")),
        ]

    def run_it(self, sh, **kw):
        cfg = ccv.Cfg(**{**self.cfg.__dict__, **kw})
        out = io.StringIO()
        r = ccv.Run(cfg, sh, out=out)
        r.probe = lambda: ((True, "6 payloads; every field nimitz reads"), (True, "doctor: 18 files carry every field"))
        r.st_internals = lambda: (True, "6 markers same as 2.1.296")
        for k, v in getattr(self, "patch", {}).items():
            setattr(r, k, v)
        self.run_obj = r
        code = r.main()
        return code, out.getvalue()


class AlreadyTested(Base):
    def test_equal_version_exits_0_and_runs_nothing_else(self):
        sh = FakeSh(self.rules(version="2.1.296"))
        code, out = self.run_it(sh)
        self.assertEqual(code, 0, out)
        self.assertIn("already tested", out)
        self.assertEqual([c for c, _ in sh.calls], ["claude --version"])

    def test_older_version_is_already_tested(self):
        code, out = self.run_it(FakeSh(self.rules(version="2.1.290")))
        self.assertEqual(code, 0)
        self.assertIn("already tested", out)

    def test_force_on_the_tested_max_validates_and_bumps_nothing(self):
        sh = FakeSh(self.rules(version="2.1.296"))
        code, out = self.run_it(sh, force=True)
        self.assertEqual(code, 0, out)
        self.assertIn("nothing to bump", out)
        self.assertFalse(sh.ran("merge") or sh.ran("push") or sh.ran("cargo install") or sh.ran("commit"), sh.calls)
        self.assertTrue(sh.ran("--test e2e"))
        self.assertIn('max = "2.1.296"', (self.repo / "docs/compat.toml").read_text())


class Checkout(Base):
    def check(self, want_code, fix, **rules):
        sh = FakeSh(self.rules(**rules))
        code, out = self.run_it(sh)
        self.assertEqual(code, want_code, out)
        self.assertIn(fix, out)
        self.assertFalse(sh.ran("worktree add") or sh.ran("nimitz status"), "nothing may run after a checkout refusal")
        return out

    def test_dirty_checkout_stops_with_the_fix(self):
        out = self.check(2, "commit or stash them", porcelain=" M README.md\n")
        self.assertIn("nothing ran", out)

    def test_other_branch_stops(self):
        self.check(2, "switch dev", branch="feat/x")

    def test_behind_origin_stops(self):
        self.check(2, "pull --ff-only origin dev", counts="0\t2")

    def test_ahead_of_origin_stops(self):
        self.check(2, "push origin dev", counts="1\t0")


class Plan(Base):
    def test_plan_lists_stages_and_runs_nothing(self):
        sh = FakeSh(self.rules())
        code, out = self.run_it(sh, plan=True)
        self.assertEqual(code, 0, out)
        for s in ("checkout", "e2e", "hooks", "formats", "internals", "bump", "merge", "on a fail"):
            self.assertIn(s, out)
        self.assertFalse(sh.ran("fetch") or sh.ran("worktree") or sh.ran("cargo") or sh.ran("nimitz status"), sh.calls)

    def test_plan_with_a_dirty_checkout_says_so(self):
        code, out = self.run_it(FakeSh(self.rules(porcelain="?? x\n")), plan=True)
        self.assertEqual(code, 2)
        self.assertIn("fix: commit or stash", out)


class PassAndFail(Base):
    def test_pass_bumps_commits_merges_pushes_installs_and_checks_doctor(self):
        sh = FakeSh(self.rules())
        code, out = self.run_it(sh)
        self.assertEqual(code, 0, out)
        wt = self.cfg.cache / "wt-2.1.297"
        commit = sh.ran("commit -q -am")
        self.assertEqual(len(commit), 1)
        self.assertIn("chore(compat): tested up to Claude Code 2.1.297", commit[0])
        self.assertTrue(sh.ran("merge --no-ff -q ci/cc-2.1.297"))
        self.assertTrue(sh.ran("push -q origin dev"))
        self.assertEqual(len(sh.ran("cargo install")), 2)
        self.assertFalse(sh.ran("plugin update"), "plugin unchanged")
        self.assertTrue(sh.ran("worktree remove --force"))
        self.assertTrue(sh.ran("branch -D ci/cc-2.1.297"))
        self.assertFalse(wt.exists())
        self.assertIn("PASS", out)
        report = (self.cfg.cache / "2.1.297" / "report.md").read_text()
        self.assertIn("(PASS)", report)
        for s in ("version", "checkout", "guard", "worktree", "build", "unit", "e2e", "hooks", "formats", "internals", "bump", "merge", "install", "doctor"):
            if s != "version":
                self.assertRegex(out, rf"✓ {s}\b")
        # the e2e guards the live fleets the snapshot saw
        e2e_env = [e for c, e in sh.calls if "--test e2e" in c][0]
        self.assertEqual((e2e_env["NIMITZ_E2E"], e2e_env["NIMITZ_E2E_LIVE_FLEETS"]), ("1", "HOME"))

    def test_bump_edits_the_four_places(self):
        r = ccv.Run(self.cfg, FakeSh([]))
        root = self.tmp / "copy"
        shutil.copytree(self.repo, root)
        r.bump(root, "2.1.296", "2.1.297")
        self.assertIn('max = "2.1.297"', (root / "docs/compat.toml").read_text())
        self.assertIn("2.1.288 - 2.1.297)", (root / "README.md").read_text())
        self.assertIn('c.max, "2.1.297"', (root / "crates/nimitz-core/src/compat.rs").read_text())
        self.assertIn('Some("2.1.298".into())', (root / "crates/nimitz-core/src/check.rs").read_text())
        with self.assertRaises(RuntimeError):
            r.bump(root, "2.1.296", "2.1.299")  # 2.1.296 is no longer there

    def test_a_plugin_change_updates_the_plugin(self):
        (self.repo / "plugin/.claude-plugin/plugin.json").write_text(json.dumps({"version": "0.4.3"}))
        sh = FakeSh(self.rules())
        code, out = self.run_it(sh)
        self.assertEqual(code, 0, out)
        self.assertTrue(sh.ran("plugin marketplace update claude-nimitz") and sh.ran("plugin update nimitz@claude-nimitz"))

    def assert_nothing_changed(self, sh):
        for k in ("commit", "merge", "push", "cargo install", "plugin update"):
            self.assertFalse(sh.ran(k), f"{k} ran on a fail: {sh.ran(k)}")
        self.assertIn('max = "2.1.296"', (self.repo / "docs/compat.toml").read_text())
        self.assertTrue(sh.ran("worktree remove --force"))
        self.assertFalse((self.cfg.cache / "wt-2.1.297").exists())

    def test_fail_in_e2e_changes_nothing_and_writes_a_report(self):
        e2e = RUN + "test e2e_07_lead_attach ... FAILED\ntest result: FAILED. 26 passed; 1 failed\n"
        sh = FakeSh(self.rules(e2e=e2e))
        code, out = self.run_it(sh)
        self.assertEqual(code, 1, out)
        self.assert_nothing_changed(sh)
        self.assertRegex(out, "✗ e2e")
        self.assertIn("FAIL", out)
        self.assertIn("cache/2.1.297/report.md", out)
        report = (self.cfg.cache / "2.1.297" / "report.md").read_text()
        self.assertIn("e2e_07_lead_attach", report)
        self.assertIn("(FAIL)", report)

    def test_a_known_unit_failure_does_not_fail_the_run(self):
        unit = RUN + "test glyphs::borders_stay_put_when_the_terminal_draws_icons_two_cells_wide ... FAILED\ntest result: FAILED. 644 passed; 1 failed\n"
        code, out = self.run_it(FakeSh(self.rules(unit=unit)))
        self.assertEqual(code, 0, out)
        self.assertIn("1 known failure", out)

    def test_an_unknown_unit_failure_fails(self):
        unit = RUN + "test x::new_bug ... FAILED\ntest result: FAILED. 644 passed; 1 failed\n"
        sh = FakeSh(self.rules(unit=unit))
        code, out = self.run_it(sh)
        self.assertEqual(code, 1, out)
        self.assert_nothing_changed(sh)

    def test_a_failed_build_skips_the_suites(self):
        rules = [("cargo build", ccv.Res(101, "error[E0308]: mismatched types\n"))] + self.rules()
        sh = FakeSh(rules)
        code, out = self.run_it(sh)
        self.assertEqual(code, 1, out)
        self.assertFalse(sh.ran("cargo test"))
        self.assertIn("not run: no build", out)
        self.assert_nothing_changed(sh)

    def test_a_live_fleet_change_is_a_failure(self):
        sh = FakeSh(self.rules())
        n = {"i": 0}
        first = json.dumps({"fleets": [{"id": "HOME", "admiral": {"id": "HOME", "role": "admiral", "session": {"session_id": "s1", "pid": 11}}}]})
        later = first.replace('"pid": 11', '"pid": 12')

        def status(cmd, cwd):
            n["i"] += 1
            return ccv.Res(0, first if n["i"] == 1 else later)

        sh.rules.insert(0, ("nimitz status --json", status))
        code, out = self.run_it(sh)
        self.assertEqual(code, 1, out)
        self.assertIn("live fleet changed", out)
        self.assert_nothing_changed(sh)
        self.assertIn("HOME: ['s1', 11] -> ['s1', 12]", (self.cfg.cache / "2.1.297" / "report.md").read_text())

    def guard_runs(self, later):
        sh = FakeSh(self.rules())
        n = {"i": 0}

        def status(cmd, cwd):
            n["i"] += 1
            return ccv.Res(0, json.dumps(STATUS) if n["i"] == 1 else json.dumps(later))

        sh.rules.insert(0, ("nimitz status --json", status))
        return sh, *self.run_it(sh)

    def test_the_guard_sees_squadrons_and_ships_and_explains_a_retire(self):
        changed = json.loads(json.dumps(STATUS))
        changed["fleets"][0]["squadrons"][0]["ships"][0]["session"]["session_id"] = "s9"
        sh, code, out = self.guard_runs(changed)
        self.assertEqual(code, 1, out)
        self.assertIn("HOME/GH/DEV", (self.cfg.cache / "2.1.297" / "report.md").read_text())
        retired = json.loads(json.dumps(STATUS))
        retired["fleets"][0]["squadrons"][0]["commodore"]["session"]["pid"] = None
        self.tearDown(); self.setUp()
        sh, code, out = self.guard_runs(retired)
        self.assertEqual(code, 0, out)
        self.assertIn("1 retired idle", out)

    def test_an_empty_status_with_fleets_on_disk_is_no_guard(self):
        (self.home / ".claude-nimitz" / "home").mkdir(parents=True)
        (self.home / ".claude-nimitz" / "home" / "fleet.toml").write_text("")
        sh = FakeSh([("nimitz status --json", ccv.Res(0, json.dumps({"fleets": []})))] + self.rules())
        code, out = self.run_it(sh)
        self.assertEqual(code, 1, out)
        self.assertIn("no members, yet fleets exist", out)
        self.assertFalse(sh.ran("worktree add"))

    def test_a_guard_change_after_the_merge_fails(self):
        sh = FakeSh(self.rules())
        n = {"i": 0}
        later = json.dumps(STATUS).replace('"pid": 13', '"pid": 99')

        def status(cmd, cwd):
            n["i"] += 1
            return ccv.Res(0, json.dumps(STATUS) if n["i"] < 3 else later)

        sh.rules.insert(0, ("nimitz status --json", status))
        code, out = self.run_it(sh)
        self.assertEqual(code, 1, out)
        self.assertTrue(sh.ran("push -q origin dev"))
        self.assertIn("changed after the merge", out)

    def test_a_crashed_test_binary_is_not_hidden_by_a_known_failure(self):
        unit = (RUN + "test glyphs::borders_stay_put_when_the_terminal_draws_icons_two_cells_wide ... FAILED\n"
                "test result: FAILED. 2 passed; 1 failed\n" + RUN + "error: process didn't exit successfully (signal: 11, SIGSEGV)\n")
        sh = FakeSh(self.rules(unit=unit))
        code, out = self.run_it(sh)
        self.assertEqual(code, 1, out)
        self.assertIn("crashed", out)
        self.assert_nothing_changed(sh)

    def test_dev_moving_but_level_with_origin_merges_nothing(self):
        sh = FakeSh(self.rules())
        n = {"i": 0}

        def dev(cmd, cwd):
            n["i"] += 1
            return ccv.Res(0, (SHA if n["i"] == 1 else "f" * 40) + "\n")

        sh.rules.insert(0, ("rev-parse dev", dev))
        code, out = self.run_it(sh)
        self.assertEqual(code, 1, out)
        self.assertIn("dev moved during the run", out)
        self.assertFalse(sh.ran("merge --no-ff") or sh.ran("push"))

    def test_a_stage_that_raises_still_removes_the_worktree(self):
        def boom():
            raise ValueError("bug")
        self.patch = {"st_e2e": boom}
        sh = FakeSh(self.rules())
        code, out = self.run_it(sh)
        self.assertEqual(code, 1, out)
        self.assertIn("ValueError: bug", out)
        self.assert_nothing_changed(sh)

    def test_a_second_run_waits_for_the_lock(self):
        self.cfg.cache.mkdir(parents=True)
        held = ccv.Run(self.cfg, FakeSh([])).take_lock()
        sh = FakeSh(self.rules())
        code, out = self.run_it(sh)
        self.assertEqual(code, 1, out)
        self.assertIn("another cc-validate is running", out)
        self.assertFalse(sh.ran("worktree"))
        held.close()

    def test_dev_moving_during_the_run_merges_nothing(self):
        sh = FakeSh(self.rules())
        n = {"i": 0}

        def counts(cmd, cwd):
            n["i"] += 1
            return ccv.Res(0, "0\t0\n" if n["i"] == 1 else "0\t1\n")

        sh.rules.insert(0, ("rev-list --left-right", counts))
        code, out = self.run_it(sh)
        self.assertEqual(code, 1, out)
        self.assertFalse(sh.ran("merge --no-ff") or sh.ran("push"))
        self.assertIn("nothing changed", out)

    def test_a_failed_push_says_the_merge_stays(self):
        sh = FakeSh([("push -q origin dev", ccv.Res(1, "rejected"))] + self.rules())
        code, out = self.run_it(sh)
        self.assertEqual(code, 1, out)
        self.assertIn("merged into dev", out)
        self.assertFalse(sh.ran("cargo install"))


class Jobs(Base):
    def job(self, short, cwd):
        d = self.cfg.claude_home / "jobs" / short
        d.mkdir(parents=True)
        (d / "state.json").write_text(json.dumps({"cwd": cwd, "state": "working"}))

    def test_drop_probe_stops_only_its_own_jobs(self):
        base = self.cfg.e2e_base / "ccv-probe-77"
        base.mkdir(parents=True)
        self.job("aaaa0001", str(base / "work"))          # launched, ours
        self.job("aaaa0002", str(base / "work"))          # its launch timed out: found by folder
        self.job("bbbb0001", "/home/u/.claude-nimitz/home/admiral")  # a live member
        self.job("bbbb0002", str(self.cfg.e2e_base / "ccv-probe-777" / "work"))  # another probe
        sh = FakeSh([])
        r = ccv.Run(self.cfg, sh)
        r.version = "2.1.297"
        r.report_dir.mkdir(parents=True)
        r.drop_probe(base, ["aaaa0001", "bbbb0001"])
        stopped = sorted(c.split()[-1] for c in sh.ran("claude stop"))
        self.assertEqual(stopped, ["aaaa0001", "aaaa0002"])
        self.assertEqual(len(sh.ran("claude rm")), 2)
        self.assertFalse(base.exists())

    def test_a_killed_e2e_has_its_own_jobs_dropped(self):
        e = self.cfg.e2e_base
        self.job("cccc0001", str(e / "nimitz-e2e-3-4242" / "ship"))
        self.job("cccc0002", str(e / "nimitz-e2e-3-42420" / "ship"))  # another pid
        self.job("cccc0003", "/home/u/work")
        sh = FakeSh([])
        sh.killed = {4242}
        r = ccv.Run(self.cfg, sh)
        r.version = "2.1.297"
        r.report_dir.mkdir(parents=True)
        r.drop_killed_e2e()
        self.assertEqual([c.split()[-1] for c in sh.ran("claude stop")], ["cccc0001"])


class Internals(Base):
    BIN = (b"\x00`bg retire ${e.record.short}: ${o.cause}, idle ${h}`\x00 var Yt=5000; xx "
           b"Ra(s,{realpath:!1,stale:Yt,retries:{retries:5,minTimeout:20},onCompromised:(g)=>t(`pins.json lock compromised (x): ${g}`)"
           b"\x00 so this started a copy as \x00 wrapped in <cross-session-message from=\"...\"> \x00"
           b'return await Ur("cli_bg_attach",k()),pX({args:["agents",...]})'
           b"\x00function Re(){let e=fk.CLAUDE_CODE_MESSAGING_SOCKET;return e}\x00")

    def binaries(self, old, new):
        self.cfg.versions.mkdir(parents=True)
        (self.cfg.versions / "2.1.296").write_bytes(old)
        (self.cfg.versions / "2.1.297").write_bytes(new)
        r = ccv.Run(self.cfg, FakeSh([]))
        r.version, r.tested = "2.1.297", "2.1.296"
        r.report_dir.mkdir(parents=True)
        return r

    def test_same_markers_under_new_minified_names_pass(self):
        new = self.BIN.replace(b"Yt", b"Qt").replace(b"Ur(", b"Zq(").replace(b"fk.", b"eT.")
        ok, summary = self.binaries(self.BIN, new).st_internals()[:2]
        self.assertTrue(ok, summary)
        self.assertIn("same as 2.1.296", summary)

    def test_a_missing_marker_fails(self):
        r = self.binaries(self.BIN, self.BIN.replace(b"started a copy as", b"made a duplicate as"))
        ok, summary = r.st_internals()[:2]
        self.assertFalse(ok)
        self.assertIn("copy text: missing", summary)

    def test_a_changed_marker_fails_with_a_diff(self):
        r = self.binaries(self.BIN, self.BIN.replace(b"Yt=5000", b"Yt=9000"))
        ok, summary = r.st_internals()[:2]
        self.assertFalse(ok)
        self.assertIn("pins lock: changed", summary)
        diff = (r.report_dir / "internals.diff").read_text()
        self.assertIn("- stale=5000", diff)
        self.assertIn("+ stale=9000", diff)

    def test_previous_binary_is_the_tested_max_else_the_newest_older(self):
        r = self.binaries(self.BIN, self.BIN)
        self.assertEqual(r.previous_binary(), "2.1.296")
        (self.cfg.versions / "2.1.296").rename(self.cfg.versions / "2.1.295")
        self.assertEqual(r.previous_binary(), "2.1.295")


class Helpers(unittest.TestCase):
    def test_versions_and_durations(self):
        self.assertTrue(ccv.newer("2.1.297", "2.1.296"))
        self.assertTrue(ccv.newer("2.1.300", "2.1.296"))
        self.assertFalse(ccv.newer("2.1.296", "2.1.296"))
        self.assertEqual(ccv.next_patch("2.1.299"), "2.1.300")
        self.assertEqual([ccv.dur(s) for s in (4, 65, 3725)], ["4s", "1m05s", "1h02m"])

    def test_payload_check_names_a_missing_field(self):
        r = ccv.Run(ccv.Cfg(*(Path("/x"),) * 6), FakeSh([]))
        need = {"SessionStart": 1, "UserPromptSubmit": 0, "PreToolUse": 0, "PermissionRequest": 0}
        seen = [{"event": "SessionStart", "raw": json.dumps({"session_id": "s", "hook_event_name": "SessionStart", "source": "startup"})}]
        ok, summary = r.check_payloads(seen, need)[:2]
        self.assertFalse(ok)
        self.assertIn("SessionStart: lacks cwd", summary)


if __name__ == "__main__":
    unittest.main()
