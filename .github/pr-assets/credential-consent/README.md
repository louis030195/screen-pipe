# Credential consent UI evidence

Captured from real Screenpipe React components with synthetic browser-runtime responses at a 1200 px viewport and 1x scale. These are browser previews, not a native Windows build or proof of DPAPI behavior.

- `grok-settings-context.png`: the real Connections screen and Grok Bot dialog, using the existing synthetic connected fixture at 1200 × 850.
- `grok-before.png`: recreated baseline `GrokBotPanel` from base commit `b247eb3c674af026fe28c628a098e9f8c748c8a5`, with a detected, not-connected Grok Bot fixture.
- `grok-after.png`: same fixture and component placement on PR implementation commit `fee7e2d03a273d1a826496184cb6f0e8887d74bb`; explicit credential disclosure and verification button.
- `grok-connected.png`: successful explicit Connect response.
- `grok-cached.png`: last-confirmed connection shown without a new credential operation.
- `grok-error.png`: explicit Connect fails; error and retry remain visible.
- `windows-browser-consent.png`: real BrowserSidebar with mocked Windows platform and a synthetic session-access event for example.com. The chat text is fixture context. Both consent actions are visible.

The Grok detail crops show the actual component in an isolated Settings / Connections preview wrapper. The temporary preview route and fixture overrides are not part of this PR.
