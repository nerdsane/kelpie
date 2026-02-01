# EVI for Full-Stack Applications

**Version:** 0.1.0
**Last Updated:** 2026-01-22
**Status:** Design Document

---

## Executive Summary

This document extends EVI to full-stack applications that include:

1. **Frontend** - React, Vue, Svelte, etc.
2. **Backend** - Rust, Node, Python, etc.
3. **Real-time** - WebSockets, SSE, etc.
4. **UI/UX** - Design systems, accessibility, user experience

EVI's core principles (Exploration, Verification, Observation, Persistence) apply, but the **specific tools and workflows** differ significantly for frontend code and UI/UX concerns.

### Key Insight: Backend vs. Frontend Verification

**Backend bugs are invisible. Frontend bugs are visible.**

- A race condition in distributed state? You'll never see it by clicking around.
- A broken button or misaligned layout? You'll see it immediately.

This means:
- **Backend** needs formal specs (TLA+) and simulation testing (DST) to find invisible bugs
- **Frontend** needs simple E2E tests and learns from production errors

Don't overengineer frontend verification. For most apps:
1. **EDN spec file** - Single source of truth for P0 constraints, states, flows
2. **Generated tests** - Tests generated from spec, not hand-written
3. **Pre-commit P0 hook** - Shows constraints at every commit
4. Production error tracking → spec update → regenerate tests

See [EVI_FULLSTACK_PIPELINES.md](./EVI_FULLSTACK_PIPELINES.md) for the pipelines and closed-loop observability.

---

## Table of Contents

1. [Full-Stack Architecture Overview](#1-full-stack-architecture-overview)
2. [Frontend Exploration](#2-frontend-exploration)
3. [UI/UX Specifications & Constraints](#3-uiux-specifications--constraints)
4. [Frontend Verification](#4-frontend-verification)
5. [Full-Stack Observability](#5-full-stack-observability)
6. [Cross-Stack Investigation](#6-cross-stack-investigation)
7. [Real-Time (WebSocket) Support](#7-real-time-websocket-support)
8. [Tool Extensions](#8-tool-extensions)
9. [Example: Kelpie with Dashboard UI](#9-example-kelpie-with-dashboard-ui)

---

## 1. Full-Stack Architecture Overview

### 1.1 Stack Layers

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Full-Stack EVI Architecture                          │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                           FRONTEND                                   │   │
│  │  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐   │   │
│  │  │Components│  │  Hooks  │  │  State  │  │ Routes  │  │  Assets │   │   │
│  │  │ (React) │  │         │  │ (Redux) │  │         │  │ (CSS)   │   │   │
│  │  └─────────┘  └─────────┘  └─────────┘  └─────────┘  └─────────┘   │   │
│  │                              │                                       │   │
│  │                              ▼                                       │   │
│  │  ┌─────────────────────────────────────────────────────────────┐   │   │
│  │  │                    WebSocket / REST API                      │   │   │
│  │  └─────────────────────────────────────────────────────────────┘   │   │
│  └──────────────────────────────┬──────────────────────────────────────┘   │
│                                 │                                           │
│  ┌──────────────────────────────┼──────────────────────────────────────┐   │
│  │                           BACKEND                                    │   │
│  │                              │                                       │   │
│  │  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐   │   │
│  │  │ Handlers│  │ Actors  │  │ Storage │  │  Auth   │  │  Jobs   │   │   │
│  │  │ (Axum)  │  │(Kelpie) │  │  (FDB)  │  │         │  │         │   │   │
│  │  └─────────┘  └─────────┘  └─────────┘  └─────────┘  └─────────┘   │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 1.2 EVI Layers for Full-Stack

| Layer | Backend (Rust) | Frontend (React) | Cross-Stack |
|-------|---------------|------------------|-------------|
| **Exploration** | Indexes, RLM | Component indexes, RLM | API contract discovery |
| **Specification** | ADRs, TLA+ ✅ Essential | Design specs (lightweight) | API contracts |
| **Verification** | DST, unit tests ✅ Essential | E2E tests, visual regression | Contract tests |
| **Observation** | Traces, metrics | Error tracking, RUM | Distributed tracing |

*Note: Backend specs (TLA+, DST) are essential because distributed bugs are invisible. Frontend specs (state machines, property tests) are optional—only for genuinely complex stateful UIs.*

### 1.3 EDN Spec Format (Single Source of Truth)

Kelpie uses **EDN (Extensible Data Notation)** for frontend specifications. EDN is a data format from Clojure—like JSON but with richer data types (sets, keywords, symbols).

**Why EDN?**
- More expressive than JSON/YAML
- Native support for keywords (`:p0`, `:inv/backend-truth`)
- Symbolic rules that can be evaluated
- Easy to parse and extract specific sections

#### The Architecture

```
specs/app.spec.edn (single source of truth)
        │
  ┌─────┼─────┬─────────┐
  │     │     │         │
  ▼     ▼     ▼         ▼
Pre-   Tests  Docs      ESLint
commit (unit, (CLAUDE.md Rules
Hook   E2E)   Storybook)
```

#### Example Spec File

```clojure
;; specs/app.spec.edn
{:spec/version "0.1.0"
 :spec/name "kelpie-dashboard"

 ;; ═══════════════════════════════════════════════════════════════════════════
 ;; P0 CONSTRAINTS - Shown at every commit via pre-commit hook
 ;; ═══════════════════════════════════════════════════════════════════════════

 :invariants
 [;; P0 - These are extracted and shown at every commit
  {:id :inv/backend-source-of-truth
   :p0 true
   :short "Backend is source of truth"
   :description "Server data MUST come from useQuery, never useState(localStorage)"
   :rule (not (and (uses? :useState)
                   (initialized-from? :localStorage)
                   (data-type? :server-data)))}

  {:id :inv/mutations-invalidate
   :p0 true
   :short "Mutations invalidate cache"
   :description "useMutation MUST invalidate relevant query cache"
   :rule (implies (mutation?) (invalidates-cache?))}

  {:id :inv/cleanup-effects
   :p0 true
   :short "Effects return cleanup"
   :description "useEffect with WebSocket/setInterval MUST return cleanup function"
   :rule (implies (effect-uses? #{:WebSocket :setInterval :EventSource})
                  (returns-cleanup?))}

  {:id :inv/no-any-types
   :p0 true
   :short "No any types"
   :description "TypeScript: No explicit 'any' types allowed"}

  {:id :inv/test-before-commit
   :p0 true
   :short "Tests pass before commit"
   :description "npm test must pass before committing"}

  ;; Non-P0 invariants (still verified, but not shown at commit)
  {:id :inv/loading-states
   :short "Show loading states"
   :description "Async operations show loading indicator"}

  {:id :inv/error-boundaries
   :short "Error boundaries exist"
   :description "Pages wrapped in error boundaries"}]

 ;; ═══════════════════════════════════════════════════════════════════════════
 ;; COMPONENT STATES - For components with non-trivial state
 ;; ═══════════════════════════════════════════════════════════════════════════

 :components
 {:actor-list
  {:states [:idle :loading :loaded :error :refreshing]
   :initial :idle
   :transitions
   [{:from :idle :to :loading :on :FETCH}
    {:from :loading :to :loaded :on :SUCCESS}
    {:from :loading :to :error :on :FAILURE}
    {:from :loaded :to :refreshing :on :REFRESH}
    {:from :refreshing :to :loaded :on :SUCCESS}
    {:from :error :to :loading :on :RETRY}]
   :invariants
   [:inv/backend-source-of-truth
    :inv/loading-states]}

  :create-actor-modal
  {:states [:closed :open :submitting :success :error]
   :transitions
   [{:from :closed :to :open :on :OPEN}
    {:from :open :to :submitting :on :SUBMIT}
    {:from :submitting :to :success :on :SUCCESS}
    {:from :submitting :to :error :on :FAILURE}
    {:from :success :to :closed :on :CLOSE :after 1000}
    {:from :error :to :open :on :RETRY}
    {:from [:open :error] :to :closed :on :CANCEL}]}}

 ;; ═══════════════════════════════════════════════════════════════════════════
 ;; USER FLOWS - Critical paths that need E2E testing
 ;; ═══════════════════════════════════════════════════════════════════════════

 :flows
 {:create-actor
  {:description "User creates a new actor"
   :steps
   [{:action :click :target "Create Actor button"}
    {:action :fill :target "Name field" :value "test-actor"}
    {:action :click :target "Submit button"}
    {:action :wait :for "Success message"}
    {:action :verify :that "Actor appears in list"}]
   :invariants [:inv/mutations-invalidate :inv/loading-states]}

  :delete-actor
  {:description "User deletes an actor"
   :steps
   [{:action :click :target "Actor menu"}
    {:action :click :target "Delete option"}
    {:action :confirm :dialog "Are you sure?"}
    {:action :wait :for "Actor removed from list"}]}}

 ;; ═══════════════════════════════════════════════════════════════════════════
 ;; DESIGN TOKENS - Visual consistency constraints
 ;; ═══════════════════════════════════════════════════════════════════════════

 :design-tokens
 {:colors
  {:primary "#3B82F6"
   :error "#EF4444"
   :success "#10B981"
   :background "#F9FAFB"
   :text "#111827"}

  :spacing
  {:xs "4px" :sm "8px" :md "16px" :lg "24px" :xl "32px"}

  :typography
  {:font-family "Inter"
   :font-family-code "JetBrains Mono"}}}
```

#### Pre-Commit Hook: P0 Extraction

The pre-commit hook extracts P0 constraints from the spec and displays them:

```bash
#!/bin/bash
# .git/hooks/pre-commit

echo "═══════════════════════════════════════════════════════════════"
echo "⛔ P0 CONSTRAINTS (NON-NEGOTIABLE)"
echo "═══════════════════════════════════════════════════════════════"

# Extract P0 invariants from spec using evi tool
evi extract-p0 specs/app.spec.edn

# Output looks like:
# 1. Backend is source of truth
#    Server data MUST come from useQuery, never useState(localStorage)
#
# 2. Mutations invalidate cache
#    useMutation MUST invalidate relevant query cache
#
# 3. Effects return cleanup
#    useEffect with WebSocket/setInterval MUST return cleanup function
#
# 4. No any types
#    TypeScript: No explicit 'any' types allowed
#
# 5. Tests pass before commit
#    npm test must pass before committing

echo ""
echo "Are you following these constraints? [y/N]"
read -r response
if [[ ! "$response" =~ ^[Yy]$ ]]; then
    echo "Commit aborted. Review P0 constraints."
    exit 1
fi

# Run tests
npm test
```

#### Generating Tests from Spec

```bash
# Generate invariant tests from spec
evi generate-tests specs/app.spec.edn --type invariant

# Generate E2E tests from flows
evi generate-tests specs/app.spec.edn --type e2e

# Generate ESLint rules from invariants
evi generate-eslint specs/app.spec.edn
```

**Generated invariant test example:**
```typescript
// GENERATED from specs/app.spec.edn
// Invariant: :inv/backend-source-of-truth

describe('INV: Backend is source of truth', () => {
  it('stale localStorage does not override server data', async () => {
    localStorage.setItem('user-1', JSON.stringify({ name: 'Stale' }))
    const { result } = renderHook(() => useUser(1), { wrapper })
    await waitFor(() => {
      expect(result.current.data.name).toBe('Test User')  // Fresh from server
    })
  })
})
```

**Generated E2E test example:**
```typescript
// GENERATED from specs/app.spec.edn
// Flow: :create-actor

test('User creates a new actor', async ({ page }) => {
  await page.click('text=Create Actor button')
  await page.fill('[data-testid="name-field"]', 'test-actor')
  await page.click('text=Submit')
  await expect(page.locator('text=Success')).toBeVisible()
  await expect(page.locator('text=test-actor')).toBeVisible()
})
```

#### When to Use State Machines vs. Simple States

The spec defines component states, but you only need XState when:
- Many states (>5) with complex transitions
- Parallel states (multiple things happening simultaneously)
- Guards (conditional transitions)
- Side effects on transitions

For simple components, the `:states` and `:transitions` in the spec are documentation + test generation input, not runtime state machines.

#### EVI CLI Tool

The `evi` command-line tool works with EDN specs:

```bash
# Extract P0 constraints (for pre-commit hook)
evi extract-p0 specs/app.spec.edn
# Output:
# 1. Backend is source of truth
#    Server data MUST come from useQuery, never useState(localStorage)
# 2. Mutations invalidate cache
#    useMutation MUST invalidate relevant query cache
# ...

# Generate tests from spec
evi generate-tests specs/app.spec.edn --type invariant  # → tests/invariants/*.test.ts
evi generate-tests specs/app.spec.edn --type e2e       # → tests/e2e/*.spec.ts
evi generate-tests specs/app.spec.edn --type component # → tests/components/*.test.tsx

# Generate ESLint rules from invariants
evi generate-eslint specs/app.spec.edn  # → .eslintrc.generated.js

# Validate spec syntax
evi validate specs/app.spec.edn

# Install pre-commit hook
evi install-hook

# Show spec summary
evi info specs/app.spec.edn
# Output:
# Spec: kelpie-dashboard v0.1.0
# P0 Invariants: 5
# Other Invariants: 2
# Components: 2
# Flows: 2
# Design Tokens: colors, spacing, typography
```

**Implementation:** The `evi` CLI is part of the `kelpie-mcp` Python package. It uses `edn_format` to parse EDN files.

---

## 2. Frontend Exploration

### 2.1 Frontend Indexes

New index types for frontend code:

| Index | Contents | Example Query |
|-------|----------|---------------|
| `components.json` | React components with props, hooks used | "Find all components using useState" |
| `hooks.json` | Custom hooks with dependencies | "What hooks fetch data?" |
| `routes.json` | Page routes and their components | "What renders at /dashboard?" |
| `stores.json` | State management (Redux, Zustand) | "What state slices exist?" |
| `styles.json` | CSS modules, styled-components | "What components use `primary` color?" |
| `assets.json` | Images, fonts, icons | "What icons are used?" |

**Example: components.json**
```json
{
  "components": [
    {
      "name": "ActorList",
      "file": "src/components/ActorList.tsx",
      "line": 15,
      "type": "functional",
      "props": ["actors", "onSelect", "loading"],
      "hooks": ["useState", "useEffect", "useActorQuery"],
      "children": ["ActorCard", "LoadingSpinner"],
      "routes": ["/dashboard", "/actors"]
    }
  ]
}
```

### 2.2 Frontend Indexer Implementation

```python
# evi/indexer/languages/typescript_react.py

class TypeScriptReactIndexer:
    """Index React/TypeScript components."""

    def extract_components(self, source: str, file_path: str) -> list[Component]:
        tree = self.parser.parse(bytes(source, "utf8"))
        components = []

        for node in self._find_nodes(tree, "function_declaration", "arrow_function"):
            if self._is_react_component(node):
                components.append(Component(
                    name=self._get_component_name(node),
                    file_path=file_path,
                    line=node.start_point[0] + 1,
                    type="functional",
                    props=self._extract_props(node),
                    hooks=self._extract_hooks(node),
                    children=self._extract_children(node),
                ))

        return components

    def extract_hooks(self, source: str, file_path: str) -> list[Hook]:
        """Extract custom hooks (functions starting with 'use')."""
        # ... implementation

    def extract_routes(self, source: str, file_path: str) -> list[Route]:
        """Extract route definitions from React Router."""
        # ... implementation
```

### 2.3 RLM for Frontend

Same pattern, different prompts:

```python
# Load React components
repl_load(pattern="src/components/**/*.tsx", var_name="components")

# Programmatic analysis for frontend
repl_exec(code="""
# Stage 1: Categorize by component type
categories = {
    'pages': [],      # Top-level route components
    'features': [],   # Feature-specific components
    'shared': [],     # Reusable UI components
    'hooks': []       # Custom hooks
}

for path in components.keys():
    if '/pages/' in path:
        categories['pages'].append(path)
    elif '/features/' in path:
        categories['features'].append(path)
    elif '/shared/' in path or '/ui/' in path:
        categories['shared'].append(path)
    elif path.startswith('use') or '/hooks/' in path:
        categories['hooks'].append(path)

# Stage 2: Targeted analysis
analysis = {}

for path in categories['pages']:
    analysis[path] = sub_llm(components[path], '''
        1. What page does this render? What route?
        2. What data does it fetch? What hooks?
        3. ISSUES: Missing loading states? Error handling? Accessibility?
    ''')

for path in categories['shared']:
    analysis[path] = sub_llm(components[path], '''
        1. What UI element is this? Props interface?
        2. Is it accessible? ARIA labels? Keyboard navigation?
        3. ISSUES: Hardcoded styles? Missing variants? TODO/FIXME?
    ''')

# Stage 3: Extract issues
issues = sub_llm(str(analysis), '''
    Extract frontend issues as JSON:
    [{"severity": "...", "description": "...", "evidence": "file:line"}]

    Frontend-specific severities:
    - critical: Security (XSS, CSRF), data exposure
    - high: Accessibility violations, missing error states
    - medium: Performance issues, missing loading states
    - low: Style inconsistencies, missing types
''')

result = {'categories': categories, 'analysis': analysis, 'issues': issues}
""")
```

---

## 3. UI/UX Specifications & Constraints

### 3.1 Design System as Specification

Like TigerStyle for backend code, we need **design constraints** for UI:

**.vision/DESIGN_SYSTEM.md**
```markdown
# Design System Constraints

## Colors (Non-Negotiable)
- Primary: `#3B82F6` (blue-500)
- Error: `#EF4444` (red-500)
- Success: `#10B981` (green-500)
- Background: `#F9FAFB` (gray-50)
- Text: `#111827` (gray-900)

## Typography
- Font: Inter
- Headings: font-semibold
- Body: font-normal
- Code: JetBrains Mono

## Spacing (8px grid)
- xs: 4px
- sm: 8px
- md: 16px
- lg: 24px
- xl: 32px

## Components (Required Patterns)
- Buttons MUST have loading state
- Forms MUST have validation errors
- Modals MUST trap focus
- Lists MUST have empty states

## Accessibility (WCAG 2.1 AA)
- Color contrast: 4.5:1 minimum
- Focus indicators: visible
- Skip links: on every page
- Alt text: on all images
```

### 3.2 UX Heuristics as Specifications

**.vision/UX_HEURISTICS.md**
```markdown
# UX Heuristics (Nielsen's + Custom)

## 1. Visibility of System Status
- Loading states for ALL async operations
- Progress indicators for multi-step flows
- Connection status for real-time features

## 2. Match Between System and Real World
- Use domain language (actors, agents, not "entities")
- Familiar icons and patterns
- Logical information hierarchy

## 3. User Control and Freedom
- Undo for destructive actions
- Cancel buttons on all modals
- Back navigation always available

## 4. Consistency and Standards
- Same action = same result everywhere
- Follow platform conventions
- Consistent terminology

## 5. Error Prevention
- Confirmation for destructive actions
- Validate input before submission
- Disable invalid actions

## 6. Recognition Rather Than Recall
- Show recent items
- Autocomplete where possible
- Contextual help

## 7. Flexibility and Efficiency
- Keyboard shortcuts for power users
- Bulk actions where appropriate
- Customizable views

## 8. Aesthetic and Minimalist Design
- Progressive disclosure
- One primary action per screen
- Remove unnecessary elements

## 9. Help Users Recover from Errors
- Clear error messages
- Suggest solutions
- Preserve user input

## 10. Help and Documentation
- Contextual tooltips
- Onboarding for new users
- Searchable documentation
```

### 3.3 Component Specifications (State Machines)

Like TLA+ for backend, use **state machines** for UI components:

**specs/ui/modal.xstate.ts**
```typescript
// Modal state machine specification
import { createMachine } from 'xstate';

export const modalMachine = createMachine({
  id: 'modal',
  initial: 'closed',
  states: {
    closed: {
      on: { OPEN: 'opening' }
    },
    opening: {
      // Animation state
      after: { 200: 'open' }
    },
    open: {
      on: {
        CLOSE: 'closing',
        SUBMIT: 'submitting',
        ESCAPE: 'closing'  // Keyboard support required
      },
      // Invariant: focus must be trapped
      meta: { focusTrapped: true }
    },
    submitting: {
      on: {
        SUCCESS: 'closing',
        ERROR: 'open'
      }
    },
    closing: {
      after: { 200: 'closed' }
    }
  }
});

// Invariants to verify:
// 1. From any state, ESCAPE should lead to closing
// 2. Focus must be trapped when open
// 3. Submitting must show loading state
```

### 3.4 New MCP Tools for UI Specs

| Tool | Purpose | Input | Output |
|------|---------|-------|--------|
| `spec_design_check` | Verify component uses design tokens | Component file | Violations |
| `spec_a11y_check` | Check accessibility compliance | Component file | WCAG violations |
| `spec_ux_check` | Check UX heuristics | Feature/page | Heuristic violations |
| `spec_state_check` | Verify state machine coverage | Component + spec | Missing states |

---

## 4. Frontend Verification

### 4.1 Verification Pyramid for Frontend

```
                    ┌─────────────┐
                    │   Manual    │  Exploratory testing
                    │   Testing   │  UX review
                    └──────┬──────┘
                           │
                    ┌──────┴──────┐
                    │     E2E     │  Playwright, Cypress
                    │    Tests    │  Critical user flows
                    └──────┬──────┘
                           │
              ┌────────────┴────────────┐
              │     Integration         │  Component + API
              │        Tests            │  React Testing Library
              └────────────┬────────────┘
                           │
         ┌─────────────────┴─────────────────┐
         │          Visual Regression         │  Chromatic, Percy
         │              Tests                 │  Storybook snapshots
         └─────────────────┬─────────────────┘
                           │
    ┌──────────────────────┴──────────────────────┐
    │              Component Unit Tests           │  Jest, Vitest
    │           Accessibility Tests               │  axe-core, jest-axe
    └─────────────────────────────────────────────┘
```

### 4.2 Frontend DST Equivalent: Property-Based UI Testing

Like DST tests backend invariants under faults, we test UI invariants under interactions:

```typescript
// tests/properties/modal.property.test.ts
import { fc } from 'fast-check';
import { render, fireEvent } from '@testing-library/react';
import { Modal } from '../components/Modal';

describe('Modal Properties', () => {
  // Property: Escape always closes modal
  it('escape key always closes modal from any state', () => {
    fc.assert(
      fc.property(
        fc.array(fc.oneof(
          fc.constant('click-open'),
          fc.constant('click-submit'),
          fc.constant('type-input'),
          fc.constant('press-escape')
        )),
        (actions) => {
          const { getByRole, queryByRole } = render(<Modal />);

          // Apply random actions
          actions.forEach(action => {
            switch (action) {
              case 'click-open':
                fireEvent.click(getByRole('button', { name: /open/i }));
                break;
              case 'press-escape':
                fireEvent.keyDown(document, { key: 'Escape' });
                break;
              // ... other actions
            }
          });

          // After escape, modal should be closed
          if (actions[actions.length - 1] === 'press-escape') {
            expect(queryByRole('dialog')).not.toBeInTheDocument();
          }
        }
      )
    );
  });

  // Property: Focus is always trapped when modal is open
  it('focus remains trapped while modal is open', () => {
    fc.assert(
      fc.property(
        fc.array(fc.constant('tab'), { minLength: 1, maxLength: 20 }),
        (tabs) => {
          const { getByRole } = render(<Modal defaultOpen />);
          const modal = getByRole('dialog');

          tabs.forEach(() => {
            fireEvent.keyDown(document, { key: 'Tab' });
          });

          // Active element should still be inside modal
          expect(modal.contains(document.activeElement)).toBe(true);
        }
      )
    );
  });
});
```

### 4.3 Visual Regression Testing

**Storybook + Chromatic workflow:**

```typescript
// stories/ActorCard.stories.tsx
import type { Meta, StoryObj } from '@storybook/react';
import { ActorCard } from '../components/ActorCard';

const meta: Meta<typeof ActorCard> = {
  title: 'Components/ActorCard',
  component: ActorCard,
  parameters: {
    // Chromatic captures snapshots of each story
    chromatic: { viewports: [320, 768, 1200] },
  },
};

export default meta;
type Story = StoryObj<typeof ActorCard>;

export const Default: Story = {
  args: {
    actor: { id: '1', name: 'TestActor', status: 'active' },
  },
};

export const Loading: Story = {
  args: {
    actor: null,
    loading: true,
  },
};

export const Error: Story = {
  args: {
    actor: null,
    error: 'Failed to load actor',
  },
};

export const LongName: Story = {
  args: {
    actor: { id: '1', name: 'Very Long Actor Name That Might Overflow', status: 'active' },
  },
};
```

### 4.4 Accessibility Testing

```typescript
// tests/a11y/ActorList.a11y.test.tsx
import { render } from '@testing-library/react';
import { axe, toHaveNoViolations } from 'jest-axe';
import { ActorList } from '../components/ActorList';

expect.extend(toHaveNoViolations);

describe('ActorList Accessibility', () => {
  it('has no accessibility violations', async () => {
    const { container } = render(
      <ActorList actors={mockActors} onSelect={jest.fn()} />
    );

    const results = await axe(container);
    expect(results).toHaveNoViolations();
  });

  it('has no violations in loading state', async () => {
    const { container } = render(
      <ActorList actors={[]} loading={true} onSelect={jest.fn()} />
    );

    const results = await axe(container);
    expect(results).toHaveNoViolations();
  });

  it('has no violations in empty state', async () => {
    const { container } = render(
      <ActorList actors={[]} onSelect={jest.fn()} />
    );

    const results = await axe(container);
    expect(results).toHaveNoViolations();
  });
});
```

### 4.5 New MCP Tools for Frontend Verification

| Tool | Purpose | Output |
|------|---------|--------|
| `verify_visual` | Run visual regression tests | Chromatic/Percy results |
| `verify_a11y` | Run accessibility tests | WCAG violations |
| `verify_e2e` | Run E2E tests | Playwright results |
| `verify_component` | Run component tests | Jest/Vitest results |
| `verify_storybook` | Check all stories render | Story errors |

---

## 5. Full-Stack Observability

### 5.1 Frontend Observability Data

| Type | Data | Tools |
|------|------|-------|
| **RUM (Real User Monitoring)** | Page load, interactions, errors | Sentry, DataDog RUM |
| **Core Web Vitals** | LCP, FID, CLS | web-vitals, Lighthouse |
| **Error Tracking** | JS errors, React error boundaries | Sentry, LogRocket |
| **Session Replay** | User session recordings | LogRocket, FullStory |
| **Analytics** | User behavior, funnels | Amplitude, Mixpanel |

### 5.2 Distributed Tracing (Frontend → Backend)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        Distributed Trace Example                             │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  trace_id: abc123                                                           │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ [Browser] user.click (button#create-actor)                          │   │
│  │ span_id: 001, duration: 2ms                                         │   │
│  └────────────────────────────────┬────────────────────────────────────┘   │
│                                   │                                         │
│  ┌────────────────────────────────┴────────────────────────────────────┐   │
│  │ [Browser] fetch POST /api/actors                                     │   │
│  │ span_id: 002, parent: 001, duration: 450ms                          │   │
│  └────────────────────────────────┬────────────────────────────────────┘   │
│                                   │                                         │
│  ┌────────────────────────────────┴────────────────────────────────────┐   │
│  │ [API Gateway] POST /api/actors                                       │   │
│  │ span_id: 003, parent: 002, duration: 445ms                          │   │
│  └────────────────────────────────┬────────────────────────────────────┘   │
│                                   │                                         │
│  ┌────────────────────────────────┴────────────────────────────────────┐   │
│  │ [Kelpie Server] actor.create                                         │   │
│  │ span_id: 004, parent: 003, duration: 400ms                          │   │
│  │                                                                      │   │
│  │   ┌──────────────────────────────────────────────────────────┐      │   │
│  │   │ [Storage] write actor state                               │      │   │
│  │   │ span_id: 005, parent: 004, duration: 350ms    ← BOTTLENECK│      │   │
│  │   └──────────────────────────────────────────────────────────┘      │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ [Browser] React setState (actors)                                    │   │
│  │ span_id: 006, parent: 002, duration: 5ms                            │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 5.3 Frontend Metrics

```typescript
// instrumentation/web-vitals.ts
import { onCLS, onFID, onLCP, onFCP, onTTFB } from 'web-vitals';

function sendToAnalytics(metric: Metric) {
  // Send to observability backend
  fetch('/api/metrics', {
    method: 'POST',
    body: JSON.stringify({
      name: metric.name,
      value: metric.value,
      id: metric.id,
      page: window.location.pathname,
    }),
  });
}

// Core Web Vitals
onCLS(sendToAnalytics);   // Cumulative Layout Shift
onFID(sendToAnalytics);   // First Input Delay
onLCP(sendToAnalytics);   // Largest Contentful Paint

// Additional metrics
onFCP(sendToAnalytics);   // First Contentful Paint
onTTFB(sendToAnalytics);  // Time to First Byte
```

### 5.4 New Observability Tools for Frontend

| Tool | Purpose | Query |
|------|---------|-------|
| `obs_rum_query` | Query Real User Monitoring data | "Page load times for /dashboard" |
| `obs_vitals_query` | Query Core Web Vitals | "LCP p95 over last 24h" |
| `obs_errors_frontend` | Query frontend errors | "JS errors on /actors page" |
| `obs_sessions_query` | Query session replays | "Sessions with rage clicks" |
| `obs_trace_frontend` | Query traces starting from browser | "Slow user interactions" |

---

## 6. Cross-Stack Investigation

### 6.1 Full-Stack Investigation Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    Full-Stack Investigation Loop                             │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  1. TRIGGER                                                                 │
│     ├─ Frontend: "Page is slow" / Error spike / Bad Web Vitals             │
│     ├─ Backend: API latency / Error rate / DST failure                     │
│     └─ Cross-stack: "Create actor takes 5 seconds"                         │
│                                                                             │
│  2. EXPLORE (both stacks)                                                   │
│     ┌─────────────────────────────────────────────────────────────────┐    │
│     │ Frontend:                                                        │    │
│     │ index_components(page="/actors")                                 │    │
│     │ repl_load(pattern="src/pages/Actors/**/*.tsx")                  │    │
│     │ sub_llm analysis: "What API calls? What happens on click?"      │    │
│     ├─────────────────────────────────────────────────────────────────┤    │
│     │ Backend:                                                         │    │
│     │ index_symbols(pattern="create_actor")                           │    │
│     │ repl_load(pattern="crates/kelpie-server/src/handlers/*.rs")     │    │
│     │ sub_llm analysis: "What does create_actor do? DB calls?"        │    │
│     └─────────────────────────────────────────────────────────────────┘    │
│                                                                             │
│  3. OBSERVE (distributed trace)                                            │
│     ┌─────────────────────────────────────────────────────────────────┐    │
│     │ obs_trace_frontend(operation="create-actor-click")               │    │
│     │ → Trace shows: 450ms total, 350ms in storage.write               │    │
│     │                                                                  │    │
│     │ obs_vitals_query(page="/actors", metric="FID")                  │    │
│     │ → FID p95: 200ms (should be <100ms)                             │    │
│     │                                                                  │    │
│     │ obs_correlate(frontend_trace, backend_trace)                    │    │
│     │ → 90% of time spent in backend storage                          │    │
│     └─────────────────────────────────────────────────────────────────┘    │
│                                                                             │
│  4. HYPOTHESIZE                                                            │
│     ├─ "Storage write is slow under load"                                  │
│     ├─ "Frontend is not showing optimistic update"                         │
│     └─ "No caching of actor list after create"                            │
│                                                                             │
│  5. VERIFY                                                                  │
│     ┌─────────────────────────────────────────────────────────────────┐    │
│     │ Frontend:                                                        │    │
│     │ verify_component(component="CreateActorButton")                  │    │
│     │ → Missing optimistic update test                                 │    │
│     │                                                                  │    │
│     │ Backend:                                                         │    │
│     │ dst_run(scenario="high_create_rate")                            │    │
│     │ → Confirms storage contention under load                         │    │
│     └─────────────────────────────────────────────────────────────────┘    │
│                                                                             │
│  6. FIX & VERIFY                                                           │
│     ├─ Frontend: Add optimistic update, show pending state                 │
│     ├─ Backend: Add write batching, improve storage performance            │
│     ├─ Add E2E test: "Create actor shows immediately"                      │
│     ├─ Add DST test: "Storage handles high create rate"                    │
│     └─ Monitor: obs_vitals_query, obs_trace_frontend                      │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 6.2 API Contract Verification

Ensure frontend and backend agree on API shape:

```typescript
// contracts/actors.contract.ts
import { initContract } from '@ts-rest/core';
import { z } from 'zod';

const c = initContract();

export const actorsContract = c.router({
  createActor: {
    method: 'POST',
    path: '/api/actors',
    body: z.object({
      name: z.string().min(1).max(100),
      type: z.enum(['agent', 'service']),
    }),
    responses: {
      201: z.object({
        id: z.string().uuid(),
        name: z.string(),
        type: z.string(),
        createdAt: z.string().datetime(),
      }),
      400: z.object({
        error: z.string(),
        details: z.array(z.string()).optional(),
      }),
    },
  },
  // ... other endpoints
});
```

**MCP Tool: Contract Verification**
```python
verify_api_contract(
    contract="contracts/actors.contract.ts",
    frontend_usage="src/api/actors.ts",
    backend_impl="crates/kelpie-server/src/handlers/actors.rs"
)
# Returns: { matches: true, mismatches: [] }
```

---

## 7. Real-Time (WebSocket) Support

### 7.1 WebSocket Message Indexing

```json
// .evi-index/websocket_messages.json
{
  "messages": [
    {
      "name": "actor.state_changed",
      "direction": "server_to_client",
      "schema": {
        "actorId": "string",
        "oldState": "ActorState",
        "newState": "ActorState"
      },
      "handlers": {
        "frontend": "src/hooks/useActorUpdates.ts:23",
        "backend": "crates/kelpie-server/src/ws/handlers.rs:45"
      }
    },
    {
      "name": "actor.invoke",
      "direction": "client_to_server",
      "schema": {
        "actorId": "string",
        "method": "string",
        "args": "any"
      },
      "handlers": {
        "frontend": "src/api/actorClient.ts:78",
        "backend": "crates/kelpie-server/src/ws/handlers.rs:102"
      }
    }
  ]
}
```

### 7.2 WebSocket Message Tracing

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        WebSocket Message Trace                               │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  [Client → Server] actor.invoke                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ { actorId: "abc", method: "process", args: { data: "..." } }        │   │
│  │ timestamp: 2026-01-22T10:00:00.000Z                                 │   │
│  │ trace_id: xyz789                                                    │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                   │                                         │
│                                   ▼                                         │
│  [Server Processing]                                                        │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ Handler: ws/handlers.rs:102                                         │   │
│  │ Duration: 150ms                                                     │   │
│  │ Actor invocation: 145ms                                             │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                   │                                         │
│                                   ▼                                         │
│  [Server → Client] actor.state_changed                                      │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ { actorId: "abc", oldState: "idle", newState: "processing" }        │   │
│  │ timestamp: 2026-01-22T10:00:00.155Z                                 │   │
│  │ trace_id: xyz789 (same trace)                                       │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 7.3 WebSocket-Specific Tools

| Tool | Purpose |
|------|---------|
| `index_ws_messages` | Index WebSocket message types and handlers |
| `verify_ws_contract` | Verify message schema matches both ends |
| `obs_ws_messages` | Query WebSocket message history |
| `obs_ws_latency` | Query WebSocket round-trip times |

---

## 8. Tool Extensions Summary

### 8.1 New Index Tools

| Tool | Purpose |
|------|---------|
| `index_components` | Index React/Vue/Svelte components |
| `index_hooks` | Index custom hooks |
| `index_routes` | Index page routes |
| `index_stores` | Index state management |
| `index_styles` | Index CSS/design tokens |
| `index_ws_messages` | Index WebSocket messages |

### 8.2 New Specification Tools

| Tool | Purpose |
|------|---------|
| `spec_design_check` | Verify design system compliance |
| `spec_a11y_check` | Check accessibility |
| `spec_ux_check` | Check UX heuristics |
| `spec_state_check` | Verify component state machines |
| `spec_contract_check` | Verify API contracts |

### 8.3 New Verification Tools

| Tool | Purpose |
|------|---------|
| `verify_visual` | Run visual regression tests |
| `verify_a11y` | Run accessibility tests |
| `verify_e2e` | Run E2E tests |
| `verify_component` | Run component tests |
| `verify_storybook` | Verify all stories |
| `verify_api_contract` | Verify API contract compliance |
| `verify_ws_contract` | Verify WebSocket contract |

### 8.4 New Observability Tools

| Tool | Purpose |
|------|---------|
| `obs_rum_query` | Query Real User Monitoring |
| `obs_vitals_query` | Query Core Web Vitals |
| `obs_errors_frontend` | Query frontend errors |
| `obs_sessions_query` | Query session replays |
| `obs_trace_frontend` | Query browser-initiated traces |
| `obs_ws_messages` | Query WebSocket history |
| `obs_ws_latency` | Query WebSocket latency |

---

## 9. Example: Kelpie with Dashboard UI

### 9.1 Project Structure

```
kelpie/
├── crates/                     # Rust backend (existing)
│   ├── kelpie-core/
│   ├── kelpie-server/
│   └── ...
│
├── dashboard/                  # React frontend (new)
│   ├── src/
│   │   ├── components/
│   │   │   ├── ActorList.tsx
│   │   │   ├── ActorCard.tsx
│   │   │   └── ...
│   │   ├── hooks/
│   │   │   ├── useActors.ts
│   │   │   └── useWebSocket.ts
│   │   ├── pages/
│   │   │   ├── Dashboard.tsx
│   │   │   └── ActorDetail.tsx
│   │   ├── stores/
│   │   │   └── actorStore.ts
│   │   └── api/
│   │       └── client.ts
│   ├── tests/
│   │   ├── components/
│   │   ├── e2e/
│   │   └── a11y/
│   └── stories/
│
├── contracts/                  # API contracts (shared)
│   ├── actors.contract.ts
│   └── websocket.contract.ts
│
├── .vision/
│   ├── CONSTRAINTS.md
│   ├── DESIGN_SYSTEM.md       # UI constraints
│   ├── UX_HEURISTICS.md       # UX rules
│   └── EVI.md
│
├── .evi-index/
│   ├── structural/            # Backend indexes
│   │   └── ...
│   ├── frontend/              # Frontend indexes
│   │   ├── components.json
│   │   ├── hooks.json
│   │   ├── routes.json
│   │   └── stores.json
│   └── contracts/             # Contract indexes
│       ├── api.json
│       └── websocket.json
│
└── CLAUDE.md                   # Updated for full-stack
```

### 9.2 Updated CLAUDE.md for Full-Stack

```markdown
# CLAUDE.md - Kelpie Development Guide

## Project Overview
Kelpie is a distributed virtual actor system with:
- **Backend:** Rust (Axum, FDB, DST)
- **Frontend:** React + TypeScript
- **Real-time:** WebSocket for actor state updates

## Quick Commands

### Backend
```bash
cargo build
cargo test
cargo test -p kelpie-dst
```

### Frontend
```bash
cd dashboard
npm run dev       # Start dev server
npm test          # Run tests
npm run storybook # Component explorer
npm run e2e       # E2E tests
```

### Full-Stack
```bash
# Start everything
docker-compose up

# Run all tests
./scripts/test-all.sh
```

## EVI Tools

### Backend Exploration
```bash
index_symbols(kind="fn", pattern="handler")
repl_load(pattern="crates/**/*.rs", var_name="backend")
```

### Frontend Exploration
```bash
index_components(pattern="ActorList")
repl_load(pattern="dashboard/src/**/*.tsx", var_name="frontend")
```

### Cross-Stack
```bash
verify_api_contract(contract="contracts/actors.contract.ts")
obs_trace_frontend(operation="create-actor")
```

## Verification Standards

### Backend
- DST tests for distributed scenarios
- Unit tests for pure functions
- Integration tests for API handlers

### Frontend
- Component tests with Testing Library
- Visual regression with Chromatic
- Accessibility tests with axe-core
- E2E tests with Playwright

### Cross-Stack
- API contract tests
- WebSocket contract tests
- Full E2E user flows
```

### 9.3 Full-Stack Skill: Investigate Slow Feature

```markdown
# .claude/skills/investigate-fullstack/SKILL.md

## When to Use
- User reports "X feature is slow"
- Web Vitals show degradation
- Error rates spike

## Workflow

### Step 1: Identify the Feature
exam_start(task="Investigate slow feature", scope=["frontend", "backend"])

### Step 2: Frontend Exploration
index_components(feature="actor-creation")
index_hooks(pattern="useCreateActor")
repl_load(pattern="dashboard/src/**/*Actor*.tsx", var_name="fe_code")
repl_exec(code="""
for path, content in fe_code.items():
    analysis[path] = sub_llm(content, '''
        1. What API calls does this make?
        2. What happens on user interaction?
        3. ISSUES: Missing loading state? Optimistic update? Error handling?
    ''')
""")

### Step 3: Backend Exploration
index_symbols(pattern="create_actor")
repl_load(pattern="crates/kelpie-server/src/handlers/*.rs", var_name="be_code")
repl_exec(code="""
for path, content in be_code.items():
    analysis[path] = sub_llm(content, '''
        1. What does this handler do?
        2. What DB operations? External calls?
        3. ISSUES: N+1 queries? Missing indexes? Blocking calls?
    ''')
""")

### Step 4: Observe
obs_trace_frontend(operation="create-actor", min_duration_ms=500)
obs_vitals_query(page="/actors", metric="FID", period="24h")
obs_correlate(frontend_traces, backend_traces)

### Step 5: Hypothesize and Verify
# Based on observations, form hypothesis
# Run targeted tests to confirm

### Step 6: Fix and Monitor
# Implement fix
# Add tests (component, DST, E2E)
# Deploy and monitor vitals
```

---

## 10. Summary

### EVI Full-Stack Extensions

| Component | Backend | Frontend | Cross-Stack |
|-----------|---------|----------|-------------|
| **Indexes** | symbols, modules, deps, tests | components, hooks, routes, stores | contracts, ws_messages |
| **Specs** | TLA+, ADRs | Design system, UX heuristics, state machines | API contracts |
| **Verification** | DST, unit, integration | Visual, a11y, E2E, component | Contract tests |
| **Observation** | Traces, metrics, logs | RUM, Web Vitals, errors, sessions | Distributed traces |

### New Tool Count

| Category | New Tools |
|----------|-----------|
| Index | 6 (components, hooks, routes, stores, styles, ws_messages) |
| Spec | 5 (design_check, a11y_check, ux_check, state_check, contract_check) |
| Verify | 7 (visual, a11y, e2e, component, storybook, api_contract, ws_contract) |
| Observe | 7 (rum_query, vitals_query, errors_frontend, sessions_query, trace_frontend, ws_messages, ws_latency) |
| **Total** | **25 new tools** |

---

*This document extends EVI for full-stack applications with frontend and UI/UX concerns.*
