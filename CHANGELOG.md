# agents-dont-sleep

## 0.2.0

### Minor Changes

- [#4](https://github.com/itsmbaqer/agents-dont-sleep/pull/4) [`1c22485`](https://github.com/itsmbaqer/agents-dont-sleep/commit/1c22485f0d0507c3b8a2cb5f70717554c0e2d480) Thanks [@itsmbaqer](https://github.com/itsmbaqer)! - Tray redesign and agent monitoring. Sessions are grouped by what needs attention, with live turn timers, the current tool, and a submenu per session: open the project folder, show its terminal app, or stop counting it. The tray icon shows when an agent needs you.
  
  New alerts tell you when an agent needs you, looks stuck, finishes a long turn, or stops with an error. You can keep awake without an agent (30 min to until you stop it), use "Sleep when agents finish", and see an Activity tab with daily stats and a 7-day chart. The hooks now record tool names, model and counts, never your prompts or code.
