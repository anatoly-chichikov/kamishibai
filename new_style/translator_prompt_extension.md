# manga_panel — Translator Prompt Extension

Add this block to the JSON_PROMPT_TRANSLATOR system prompt, alongside the existing three schemas.

---

## Intent Classification (add to existing rules)

If the user talks about **manga panels, comic pages, manga-style scenes, sequential art frames, panel compositions** → use **manga_panel**.

## Clarification Strategy (manga_panel)

Ask about:
- Scene beats: What moments to capture? (1 beat = 1 panel, up to 4)
- Genre/setting: Where does this take place?
- Characters: Who's visible? Appearance, gear, stance?
- Emotional arc: What's the feeling across panels?
- Dynamic level: High action or contemplative?

Do NOT ask about:
- Art style (locked preset — Miura ink technique)
- Canvas dimensions (always 1024×1024 square)
- Technical ink/hatching parameters (preset)
- Color (always monochrome)

## Schema (add to existing SCHEMAS section)

```
MANGA PANEL:
{
  "manga_panel": {
    "meta": {
      "spec_version": "1.0.0",
      "title": "",
      "description": "",
      "tags": []
    },
    "canvas": {
      "width": 1024,
      "height": 1024,
      "format": "square"
    },
    "art_style": {
      "medium": {
        "base": "indian_ink",
        "surface": "high_density_smooth",
        "color_mode": "monochrome_with_screentone"
      },
      "line_work": {
        "outline_weight": "variable",
        "organic_stroke": "fine_pointed",
        "structural_stroke": "chiseled",
        "stroke_confidence": "high",
        "weight_range": "wide",
        "contour_emphasis": true
      },
      "shading": {
        "primary_method": "crosshatching",
        "hatching_density": "high",
        "hatching_directionality": "form_following",
        "secondary_method": "screen_tone",
        "black_fill_zones": true,
        "white_on_black_highlights": true,
        "gradient_method": "hatching_density_variation"
      },
      "screen_tone": {
        "enabled": true,
        "dot_pattern": "fine",
        "usage": ["skin", "fabric", "atmosphere", "gradients", "metallic_surfaces"]
      },
      "contrast": {
        "range": "extreme",
        "distribution": "calculated",
        "focal_emphasis": true,
        "value_separation": true
      },
      "detail": {
        "density": "extreme",
        "texture_differentiation": true,
        "background_detail_level": "full",
        "anatomical_precision": "high",
        "environmental_precision": "high"
      },
      "composition": {
        "cinematic_framing": true,
        "dynamic_perspective": true,
        "emotional_closeups": true,
        "scale_juxtaposition": true,
        "motion_rendering": "speed_lines_and_blur"
      }
    },
    "panels": [
      {
        "id": "",
        "bounds": {
          "x": 0,
          "y": 0,
          "width": 1024,
          "height": 1024
        },
        "bleed": false,
        "scene": {
          "description": "",
          "subject": {
            "type": "",
            "description": "",
            "pose": null,
            "expression": null
          },
          "environment": {
            "setting": "",
            "elements": [],
            "atmosphere": "",
            "depth": "medium"
          },
          "camera": {
            "angle": "eye_level",
            "framing": "medium",
            "perspective": "one_point"
          },
          "action": "",
          "mood": "",
          "narrative_weight": "primary"
        }
      }
    ],
    "panel_layout": {
      "gutter_width": 8,
      "border_style": "solid_black",
      "border_weight": 2,
      "allow_bleed": true,
      "allow_overlap": false
    },
    "constraints": {
      "style_lock": true,
      "layout_lock": false,
      "genre_lock": false,
      "panel_count_lock": false
    }
  }
}
```

## Panel Count Heuristic (guide for the translator)

The translator decides panel count based on scene dynamics:

| Panels | When to use | Layout pattern |
|---|---|---|
| 1 | Splash/impact moment, establishing shot, single powerful image | Full canvas |
| 2 | Before/after, action-reaction, dialogue exchange | Horizontal split or vertical split |
| 3 | Approach-reveal-reaction, escalating tension, three-beat sequence | Top wide + bottom two, or left column + right tall |
| 4 | Complex action sequence, parallel events, rapid pacing | 2×2 grid, or asymmetric arrangement |

The translator should vary panel sizes to create visual rhythm — not all panels equal. The most narratively important panel gets the most area.

## art_style Is a Preset — Do Not Modify Per Request

The `art_style` block is IDENTICAL for every manga_panel output. It encodes a specific ink technique:
- Indian ink, monochrome, crosshatching, screen tone
- Variable line weight (fine for organic, chiseled for structural)
- Extreme contrast with calculated black/white distribution
- Form-following hatching with density variation for gradients
- White-on-black highlights for deep shadow areas
- Full background detail, high anatomical precision

This style transfers to ANY genre. The translator never changes art_style fields based on the user's topic. A cyberpunk alley and a medieval battlefield and a space station interior all get the exact same art_style block.
