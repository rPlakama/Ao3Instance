---
name: tasks-queue
description: Manage multi-step or multi-request work as a persistent task queue instead of a flat to-do list. Use this whenever the user gives a new task while another is still in progress, sends a correction or additional request mid-task, says things like "also do X", "actually, first do Y", "add this to the list", or asks what's pending/next. Trigger even if the user doesn't say the word "queue" — any interruption, reprioritization, or stacking of work during an active task should route through this skill. Do NOT treat a queue update as a reason to stop, abandon, or restart the task currently in progress.
---

# Tasks Queue

## Core rule
Never stop the current task because something new arrived. A new or changed task only edits the **queue** — it does not interrupt execution. Finish (or reach a natural checkpoint in) the active item, then pull the next one from the queue in order.

## Queue structure
Maintain a simple ordered list, each item with:
- `id` — short stable identifier
- `status` — `active`, `queued`, `done`, `cancelled`
- `desc` — one-line description of the task
- `note` — optional context (why it was added, dependencies, priority flag)

Only one item is `active` at a time.

## Handling events
- **New task arrives while one is active** → append to the end of the queue as `queued`. Do not switch. Acknowledge briefly ("added to the queue") and keep working on the active item.
- **User flags something urgent** → move it near the front of the queue (not necessarily position 1 if something is mid-flight), but still don't abandon the active item unless the user explicitly says to stop/switch now.
- **User changes or cancels a queued item** → edit or mark it `cancelled` in place; this never touches the active item.
- **User changes the active item itself** (new constraints, scope change) → that's a live edit to the active task, applied immediately — distinct from queue management.
- **Active item finishes** → mark `done`, promote the next `queued` item to `active`, state what's starting.

## Output conventions
- Keep the queue visible/updatable on request ("what's left?", "show the queue") — render it as a short ordered list with status markers.
- When adding or reordering, state the change in one line rather than re-printing the whole queue unless asked.
- Don't pad the queue with speculative tasks the user didn't actually ask for.

## Anti-patterns to avoid
- Dropping the active task to "quickly" do the new one.
- Silently reordering without telling the user why.
- Losing queued items between turns — always carry the current queue state forward explicitly.
