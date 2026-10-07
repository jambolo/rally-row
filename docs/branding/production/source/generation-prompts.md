# Rally Row production generation prompts

An AI image generator produced the refinement references. The final production
assets were rebuilt as vector paths because the generated transparent images
had edge artifacts. The navy-background treatment is primary.

These prompts are kept for the record. For current color and typography roles, see
[the production artwork notes](../README.md). The initial refinement used the
secondary navy-on-light treatment; the correction restored the primary off-white,
blue, and coral treatment for navy backgrounds.

## Initial refinement

```text
Use case: logo-brand
Asset type: isolated final logo artwork, transparent PNG, wide horizontal layout.
Input image 1 is the approved Rally Row brand reference. Extract and refine its identity into one clean logo lockup; do not create a presentation board.
Primary request: Preserve the approved abstract R with three separated parallel seating-row forms: off-white upper R bowl and right diagonal leg, electric-blue middle bent row, coral short lower bent row. Preserve the bold, condensed, forward-slanted uppercase RALLY ROW lettering. Match the geometry and personality of the reference as closely as possible, with precise smooth contours and flat fills.
Composition: compact abstract R on the left, RALLY ROW wordmark on the right, horizontally aligned. Beneath the wordmark include the selected tagline, exactly "A little insight. A lot to talk about." in a clean upright sans serif. Give the logo generous transparent margins. One logo only.
Color palette: upper mark and wordmark #101B2D (navy for light surfaces), middle row #2667FF, lower row #FF6B5E, tagline #101B2D.
Background: genuinely transparent alpha, no panel, no shadow, no fake checkerboard. High resolution with crisp flat edges.
Text verbatim: "RALLY ROW" and "A little insight. A lot to talk about."
Constraints: faithful refinement of the supplied logo; no new symbol, sport equipment, outline, registration or trademark symbol, badges, gradients, shadows, textures, glow, mockups, captions, swatches, or sample matchup data. This is finished standalone logo artwork.
```

## Primary color correction

```text
Use case: precise-object-edit
Asset type: primary Rally Row logo artwork for navy backgrounds, transparent PNG.
Input image is the isolated Rally Row logo lockup. Change ONLY the navy upper R shape, navy RALLY ROW wordmark, and navy tagline to flat off-white #F5F7FB. Preserve their exact geometry, placement and size. Keep the middle stripe electric blue #2667FF and bottom stripe coral #FF6B5E.
Keep the transparent background genuinely transparent. This white/blue/coral logo will be placed on midnight navy #101B2D in its primary use.
Keep exact text "RALLY ROW" and "A little insight. A lot to talk about."
Remove any texture or gradient in fills; use only the three exact solid foreground colors and edge antialiasing. Do not add any panel, background, shadow, new text, or new logo. Preserve the existing logo and wording exactly.
```
