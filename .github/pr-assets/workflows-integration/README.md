# Workflows integration visual evidence

All screenshots use the real React components with fictional browser-mock data.
No customer recordings, account data, or real assistant responses are included.

The recreated baseline uses the main Home component from parent PR #6865 at
`e5b2cb29ca717dadcd4dfa481c0640ee00c3ce29`, with the mock onboarding-status response
corrected so the first-run hook can load. The before/after main-app comparison
uses a 1280 × 720 viewport and the same synthetic chat fixtures in dark mode.
Relative chat ages advance with the preview clock. The baseline includes the
Next.js development badge; it was hidden for the subsequent captures.

| Image | State |
| --- | --- |
| 01-before-main.png | Parent main navigation before the workspace menu |
| 02-workspace-switcher.png | Chat / Workflows menu open |
| 03-first-workflow-empty.png | First entry, no catalog yet, explicit build action |
| 04-workflow-catalog.png | Fictional catalog inside the main application |
| 05-workflow-and-assistant.png | Workflow detail and fixture assistant response |
| 06-left-sidebar-collapsed.png | Navigation collapsed, assistant and draft retained |
| 07-both-sidebars-collapsed.png | Both panes collapsed, reopening controls visible |
| 08-keyboard-shortcuts.png | Command menu with Cmd+B and Option+Cmd+B |
| 09-main-with-workspace-menu.png | Main navigation after integration |
| 10-onboarding-first-task.png | Production first-task component in the maintained mock harness |
| 11-onboarding-workflow-focus.png | Optional workflow focus, examples, and skip |
| 12-compact-work-profile.png | Work profile at 900 × 720 with navigation collapsed |

Onboarding placement: existing login → acquisition (consumer) → permissions →
optional timeline choice → engine → plan when required → recommended setup →
first-task choice. Managed deployments and summary-first treatment retain their
existing flow. The onboarding images use `/dev/workflows-ux`, which intentionally
shows its fixture controls; they are component evidence, not a native permission
or checkout walkthrough.

The workflow canvas retains the parent PR's light palette. The host menu,
onboarding, and assistant use their existing theme handling.

Reproduce with `bun run dev:web` in `apps/screenpipe-app-tauri`, then open `/home`.
Select Workflows from the workspace menu and use Build my workflow catalog to
load synthetic history. `/dev/workflows-ux` provides the first-task and history
states without replaying native permissions. It is blank outside mock mode.

Manual verification covered workspace round trips, preserved drafts in both
chats, independent pane toggles, the correct command menu in the active mode,
profile navigation, and the 900px layout. macOS key events were exercised in the
browser. Windows/Linux modifier semantics are covered by unit tests; their
native windows have not been exercised for this PR.
