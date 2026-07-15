# Redo Decision 2026-07-14. Supersedes engine/apps/decision (targeted-rebuild),
# whose premises predated the 2026-07-14 drift verdict: the targeted/continue
# path IS what failed. Same Kepner-Tregoe fact types, population updated to the
# new evidence, plus the deontic choice gate the earlier model lacked: the
# elimination is DERIVED, and apply must REFUSE any eliminated choice.

## Entity Types

Option(.name) is an entity type.
Must(.name) is an entity type.
Rebuild(.name) is an entity type.

## Readings

Option is under consideration.
Option fails Must.

Option is eliminated.
* Option is eliminated iff that Option fails some Must.

Option is viable.
* Option is viable iff that Option is under consideration and that Option is not eliminated.

Rebuild is planned.
Rebuild uses Option.

## Constraints

Each Rebuild uses at most one Option.
Each Rebuild uses some Option.
For each Option, at most one of the following holds: that Option is eliminated; some Rebuild uses that Option.

## Facts — the options

Option 'greenfield-transcribe' is under consideration.
Option 'salvage-assembly' is under consideration.
Option 'strangler-in-place' is under consideration.
Rebuild 'redo-2026-07' is planned.

## Facts — the musts each option fails

# source-anchored: every change judged against the sources (whitepaper, NORMA,
#   iFactr, Backus/Codd), never the previous patch (patch myopia = the drift
#   mechanism) nor artifact quality (clean parts reassemble the drifted whole)
# breaks-failed-loop: must not reuse the failed keep-alive patch loop
#   (2026-07-14 verdict; cron d53e0990 stopped)
# fits-week: executable within the Fable 5 window (~1 week from 2026-07-14)
# preserves-certified-work: the rho-certified canon (345/362 pure) re-enters
#   through certification gates, not burned (asserted against no option:
#   greenfield-transcribe salvages through gates, unlike the earlier full-redo)

Option 'strangler-in-place' fails Must 'breaks-failed-loop'.
Option 'strangler-in-place' fails Must 'source-anchored'.
Option 'salvage-assembly' fails Must 'source-anchored'.
