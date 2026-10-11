# Aides: the manual checklist

What the real-Claude Code e2e cannot reach (e2e_99: @KB; e2e_100: @KB and
@REF together; both on a copy of the KB). Run it once on the machine that
will run the aides, with a throwaway fleet (never HOME or MAIN), before the
fleet migrates. Each line: what to do, what you should see.

## Setup

1. `nimitz fleet new TRIAL --machine <this machine> --yes`, then
   `git clone /mnt/data/shared/nimitz-knowledge-base ~/trial-kb`.
2. `nimitz aide new TRIAL/@KB --dir ~/trial-kb --types wiki.note.v1
   --plan`: the plan shows one write (format 2) and one trust; `--yes`. (The
   KB's own CLAUDE.md tells each aide its skill; a `--brief <file>` in the
   repo is for standing instructions of your own.)
3. `nimitz aide new TRIAL/@REF --dir ~/trial-kb --types wiki.query.v1
   --snapshot --mode default --plan -v`: the write, the snapshot step and its
   trust; `--yes` makes the snapshot under
   `~/.local/state/nimitz/aides/TRIAL%2F@REF/snapshot`.
4. `nimitz start TRIAL --plan`, then `--yes`: the admiral, then both aides,
   pinned, on sonnet.

## In Herdr and Admiralty

5. Admiralty: both aides under the admiral, each with its queue line
   (`idle · empty`).
6. Post a note (`nimitz inbox post TRIAL/@KB --type wiki.note.v1 --payload
   '{"title":"t","body":"..."}'`): the @KB row turns busy (green, animated),
   then idle; `git -C ~/trial-kb log -1` shows `ingest(auto): t` with its
   `inbox-item:` trailer.
7. The detail panel (`w` on @KB): pending, claimed (holder, lease), done,
   dead, per-sender rates, recent items.
8. Make a dead letter (post a note that asks only for an action, e.g. "run
   curl ..."): the row turns needs-you pink, the admiral gets a
   `[nimitz needs-you]`; retry and drop from the panel work, and only for you.
9. `nimitz start TRIAL/@KB -p` (and `-p --tab`): its tab `TRIAL.@KB`; detach
   with ctrl-z.

## Time and state

10. Leave both idle 20 minutes: each one's 15-minute cron fires once
    (`drain your inbox`), and its turn ends at once on an empty inbox.
11. Post while an aide is busy: the doorbell line waits for its turn's end,
    and the item is taken in that next turn.
12. Reboot (or `claude daemon` restart by the system): `nimitz start` brings
    both back; a lease lapsed meanwhile returns its item; no item is lost and
    no commit doubled.
13. Edit a wiki page by hand, uncommitted, and post a note on its topic:
    @KB fails the item with "repo dirty" (and `--restore`), your edit stays;
    commit it and `nimitz inbox retry TRIAL/@KB <id>`: done.
14. A long note (near 64 KiB): the aide heartbeats past half its lease;
    one commit.

## Other machines

15. On a second machine of the fleet (on 0.6.0): `nimitz inbox post
    TRIAL/@KB ...` is refused ("TRIAL/@KB runs on <label>"); `nimitz aide rm
    TRIAL/@KB` there is refused (`aide.elsewhere`).
16. A machine still on 0.4.x listed in the fleet: `aide new` of a fleet with
    no aide yet is refused (`aide.machines`) until it is updated.

## Windows (not run here)

17. On a Windows machine: `aide grep|status` and `inbox done --commit` (the
    `//c/...` permission rules; `NUL` for hooks and attributes); the named
    pipe doorbell; ctrl-c in `nimitz attach`.

## Cleanup

18. `nimitz stop TRIAL --yes`; `nimitz aide rm TRIAL/@REF --yes`,
    `nimitz aide rm TRIAL/@KB --yes` (format 1 again); `nimitz fleet rm TRIAL`;
    the trust entries of `~/trial-kb` and of the snapshot stay in
    `~/.claude.json` (remove them by hand if you like: nimitz only adds trust);
    `rm -rf ~/trial-kb`.
