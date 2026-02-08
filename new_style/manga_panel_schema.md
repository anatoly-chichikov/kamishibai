# manga_panel — JSON Schema for Manga Panel Image Generation

## Overview

This schema defines a **manga panel composition** rendered inside a square canvas. The canvas contains 1–4 panels (tiles), each with its own scene. The art style is encoded as **technical drawing properties** — medium, ink technique, hatching, contrast, line work — divorced from any specific genre or IP. This allows the same style spec to produce consistent visuals whether the scene is cyberpunk, sci-fi, historical, or anything else.

---

## Schema Structure

```
{
  "manga_panel": {
    "meta": { ... },
    "canvas": { ... },
    "art_style": { ... },
    "panels": [ ... ],
    "panel_layout": { ... },
    "constraints": { ... }
  }
}
```

---

## Field Reference

### meta

| Field | Type | Required | Description |
|---|---|---|---|
| spec_version | string | yes | Schema version, e.g. "1.0.0" |
| title | string | yes | Internal name for this composition |
| description | string | no | Brief summary of the scene/purpose |
| tags | string[] | no | Keywords for cataloging |

### canvas

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| width | number | yes | 1024 | Canvas width in px |
| height | number | yes | 1024 | Canvas height in px |
| format | string | yes | "square" | "square" only for this spec |

### art_style

This is the core of the spec. It encodes **drawing technique** — not genre, not theme, not IP.

#### art_style.medium

| Field | Type | Required | Description |
|---|---|---|---|
| base | string | yes | Primary medium: "indian_ink", "brush_ink", "digital_ink" |
| surface | string | no | Paper type implied: "high_density_smooth", "light_grain" |
| color_mode | string | yes | "monochrome_bw" or "monochrome_with_screentone" |

#### art_style.line_work

| Field | Type | Required | Description |
|---|---|---|---|
| outline_weight | string | yes | "variable" — thin for organic, thick for structural |
| organic_stroke | string | yes | Nib style for skin, hair, fabric: "fine_pointed" |
| structural_stroke | string | yes | Nib style for architecture, armor, hard surfaces: "chiseled" |
| stroke_confidence | string | yes | "high" — deliberate, no sketchy lines |
| weight_range | string | no | "wide" — from hairline to heavy fill strokes |
| contour_emphasis | boolean | no | true — strong outlines on figures/objects |

#### art_style.shading

| Field | Type | Required | Description |
|---|---|---|---|
| primary_method | string | yes | "crosshatching" |
| hatching_density | string | yes | "high" — dense, layered strokes |
| hatching_directionality | string | yes | "form_following" — hatching follows 3D surface curvature |
| secondary_method | string | no | "screen_tone" |
| black_fill_zones | boolean | yes | true — large areas of solid black for deep shadow |
| white_on_black_highlights | boolean | yes | true — white ink/erased highlights over filled black |
| gradient_method | string | no | "hatching_density_variation" — lighter = sparser hatching |

#### art_style.screen_tone

| Field | Type | Required | Description |
|---|---|---|---|
| enabled | boolean | yes | true |
| dot_pattern | string | no | "fine" — small dots, high DPI feel |
| usage | string[] | yes | Where to apply: "skin", "fabric", "atmosphere", "gradients", "metallic_surfaces" |

#### art_style.contrast

| Field | Type | Required | Description |
|---|---|---|---|
| range | string | yes | "extreme" — deep blacks to pure whites, minimal midtone |
| distribution | string | yes | "calculated" — intentional light/dark zone planning |
| focal_emphasis | boolean | yes | true — highest contrast at narrative focal point |
| value_separation | boolean | no | true — distinct value planes (foreground/mid/back) |

#### art_style.detail

| Field | Type | Required | Description |
|---|---|---|---|
| density | string | yes | "extreme" — every surface has texture |
| texture_differentiation | boolean | yes | true — unique hatching pattern per material |
| background_detail_level | string | yes | "full" — backgrounds as detailed as foreground |
| anatomical_precision | string | yes | "high" — musculature, bone structure visible |
| environmental_precision | string | no | "high" — architecture, vegetation, debris rendered precisely |

#### art_style.composition

| Field | Type | Required | Description |
|---|---|---|---|
| cinematic_framing | boolean | yes | true — film-like angles and shot choices |
| dynamic_perspective | boolean | yes | true — dramatic foreshortening, low/high angles |
| emotional_closeups | boolean | no | true — extreme close-ups on face for emotion beats |
| scale_juxtaposition | boolean | no | true — small figure vs massive environment/creature |
| motion_rendering | string | no | "speed_lines_and_blur" — action conveyed through line |

### panels (array, 1–4 items)

Each panel is a tile on the canvas.

| Field | Type | Required | Description |
|---|---|---|---|
| id | string | yes | Unique panel ID: "p1", "p2", etc. |
| bounds | object | yes | Position and size on canvas |
| bounds.x | number | yes | X offset from top-left |
| bounds.y | number | yes | Y offset from top-left |
| bounds.width | number | yes | Panel width in px |
| bounds.height | number | yes | Panel height in px |
| bleed | boolean | no | If true, panel art extends to canvas edge |
| scene | object | yes | What's depicted in this panel |

#### panels[].scene

| Field | Type | Required | Description |
|---|---|---|---|
| description | string | yes | Full scene description in natural language |
| subject | object | no | Primary figure/object |
| subject.type | string | no | "character", "creature", "vehicle", "object", "environment_only" |
| subject.description | string | no | Physical appearance, gear, posture |
| subject.pose | string | no | Action or stance |
| subject.expression | string | no | Emotional state visible on face |
| environment | object | no | Setting details |
| environment.setting | string | no | Where the scene takes place |
| environment.elements | string[] | no | Specific props/details in frame |
| environment.atmosphere | string | no | Mood of the environment itself |
| environment.depth | string | no | "shallow", "medium", "deep" — sense of space |
| camera | object | no | Virtual camera |
| camera.angle | string | no | "low", "eye_level", "high", "birds_eye", "worms_eye", "dutch" |
| camera.framing | string | no | "extreme_closeup", "closeup", "medium", "wide", "extreme_wide" |
| camera.perspective | string | no | "flat", "one_point", "two_point", "dramatic_foreshortening" |
| action | string | no | What's happening in the moment |
| mood | string | no | Emotional tone: "tension", "melancholy", "rage", "awe", etc. |
| narrative_weight | string | no | "primary", "secondary", "transition" — importance in sequence |

### panel_layout

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| gutter_width | number | no | 8 | Space between panels in px |
| border_style | string | no | "solid_black" | Panel border style |
| border_weight | number | no | 2 | Border thickness in px |
| allow_bleed | boolean | no | true | Can panel art break borders |
| allow_overlap | boolean | no | false | Can panels overlap each other |

### constraints

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| style_lock | boolean | yes | true | Art style must not change |
| layout_lock | boolean | no | false | Panel arrangement is fixed |
| genre_lock | boolean | no | false | Genre/setting is fixed |
| panel_count_lock | boolean | no | false | If true, panel count is fixed |

---

## Translator Rules for manga_panel

### Intent classification
If the user asks for **manga panels, comic pages, sequential art, manga-style scenes** → use `manga_panel`.

### Clarification questions
Ask about:
- **Scene(s)**: What's happening? How many moments/beats?
- **Genre/setting**: Cyberpunk? Historical? Sci-fi? Fantasy? Slice of life?
- **Characters**: Who's in frame? Appearance?
- **Mood/tone**: Tense? Calm? Explosive? Melancholic?
- **Dynamic level**: High action (more panels) or contemplative (fewer, larger panels)?

Do NOT ask about:
- Art style details (these are locked as the Miura-technique preset)
- Canvas size (always 1024×1024)
- Technical ink parameters (preset)
