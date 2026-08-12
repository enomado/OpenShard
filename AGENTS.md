# Agent execution rules

## Keep working until a real terminal condition

- A progress update is not a terminal condition. Put it in the commentary
  channel, then immediately perform the next safe planned action.
- Do not send a final response while a concrete, reversible implementation,
  validation, or inspection step remains. A final response ends the turn even
  if it says "I will continue".
- End only when the requested outcome is complete, the user asks to stop, or a
  material decision/external authority is genuinely required. State the exact
  blocker in that last case.
- When a worklist has a clear next item, take it autonomously. Do not ask for
  confirmation or substitute an announcement for the work.
