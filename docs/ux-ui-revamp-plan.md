# Hatchdoor UX/UI Revamp Plan

## Status
- Current state: MVP is functional (Rust API + React/Vite frontend + PWA scaffolding).
- Goal: full UX/UI revamp with mobile-first quality and Obsidian-like reading experience.

## Phase 1: Foundation + Reading UX (Merged)

### Milestone A: Navigation + Mobile Foundation
1. Implement responsive app shell
- Desktop: resizable sidebar + content pane.
- Mobile: explorer drawer with open/close controls.
- Persist sidebar width and drawer state.

2. Add robust UI states
- Skeleton loading for tree and note content.
- Distinct states: empty, not found, load error, stale/offline.

### Milestone B: Reading Experience
1. Reading controls
- Text size, line height, content width presets.
- Sticky top bar with note path + close/back actions.

2. Markdown polish
- Styled callouts (`[!note]`, `[!warning]`, etc.).
- Improved code blocks (copy button, language badge).
- Better unresolved-link visual treatment.

### Acceptance Criteria
1. iPhone viewport can open/close explorer and select notes without layout break.
2. Desktop sidebar resize persists across reload.
3. No blank screens during loading/error states.
4. Reading controls apply instantly and persist.
5. Callouts and code blocks are visually distinct and usable on mobile.
6. Broken links are clearly visible and consistent.

## Phase 2: Search + Discovery

### Work
1. Global search UI
- Keyboard shortcut entry.
- Mobile-friendly search entry.
- Search by title/path first, then content (phase extension).

2. History + recent notes
- Recent notes list.
- Back/forward note navigation behavior.

### Acceptance Criteria
1. Search opens target note in <= 2 interactions.
2. Recent notes update reliably.
3. Back/forward works for note route transitions.

## Phase 3: Accessibility + Performance

### Work
1. Accessibility pass
- Landmark roles and heading hierarchy.
- Keyboard navigation for tree/search/results.
- Focus rings and contrast improvements.

2. Performance pass
- Explorer tree virtualization for large vaults.
- Prefetch likely next note targets.
- Keep heavy renderer code split (Mermaid done; extend if needed).

### Acceptance Criteria
1. App is fully usable via keyboard only.
2. Explorer and note transitions remain responsive on large vaults.
3. Mobile performance metrics improve materially.

## Phase 4: Design System + Final QA

### Work
1. Tokenized design system
- Color, spacing, typography, radius, and elevation tokens.
- Reusable components: button, panel, badge, toolbar, empty state.

2. QA matrix
- Desktop: Chrome, Firefox, Safari.
- Mobile: iPhone Safari + installed PWA behavior.
- Offline/reconnect validation.

### Acceptance Criteria
1. Visual consistency across major screens and breakpoints.
2. PWA install/resume behavior is stable.
3. No critical regressions in rendering or navigation.

## Recommended Execution Order
1. Phase 1 (Milestone A then Milestone B in the same branch)
2. Phase 2
3. Phase 3
4. Phase 4

## Notes
- Keep backend API contracts stable while improving UX incrementally.
- Validate each phase with explicit before/after screenshots and checklist sign-off.
