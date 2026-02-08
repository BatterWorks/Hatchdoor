# Phase 4 QA Matrix

## Scope
- Tokenized design system baseline:
  - color, spacing, radius, elevation primitives
  - reusable primitives: button, panel, badge, toolbar, empty state
- Final cross-platform validation for Hatchdoor MVP + Phase 2/3 behaviors

## Browsers
| Area | Browser | Status |
|---|---|---|
| Desktop | Chrome (latest) | Pending |
| Desktop | Firefox (latest) | Pending |
| Desktop | Safari (latest) | Pending |
| Mobile | iPhone Safari | Pending |
| Mobile | iPhone PWA installed | Pending |

## Core Flows
| Flow | Expected Result | Status |
|---|---|---|
| Explorer open/close | Drawer works on mobile, sidebar stable on desktop | Pending |
| Folder tree | Collapsed by default, active-note chain opens, manual toggles persist | Pending |
| Note navigation | Open note from tree/recent/search; back/forward/close behave correctly | Pending |
| Search (title/path) | Query opens relevant note in <=2 interactions | Pending |
| Search (content) | `Include content matches` returns snippet hits | Pending |
| Recent notes | List updates after note opens and persists across reload | Pending |
| Markdown rendering | Headings, lists, code blocks, callouts, math, mermaid render correctly | Pending |
| Broken links | Missing wikilinks are visibly distinct and non-clickable | Pending |
| Offline state | Offline badge appears, cached UI remains usable | Pending |
| Vault refresh | Tree/note updates appear after backend refresh interval | Pending |

## Responsive Checks
| Viewport | Check | Status |
|---|---|---|
| 390x844 (iPhone 12) | Topbar actions fit without overlap | Pending |
| 390x844 (iPhone 12) | Explorer background opaque, readable note titles | Pending |
| 768x1024 (tablet) | Drawer transitions and content spacing remain usable | Pending |
| >=1280px desktop | Sidebar resize, toolbar, search modal spacing are stable | Pending |

## PWA Checks
| Check | Expected Result | Status |
|---|---|---|
| Install prompt / manual add | App installs successfully | Pending |
| Relaunch from home screen | App opens in standalone with valid layout | Pending |
| Service worker update | New build activates without data loss | Pending |
| Offline relaunch | Shell loads and shows appropriate offline indicators | Pending |

## Regression Guardrails
- Backend:
  - `cargo fmt --all --check`
  - `cargo check`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- Frontend:
  - `npm run lint`
  - `npm run format:check`
  - `npm run typecheck`
  - `npm run test -- --run`
  - `npm run build`

