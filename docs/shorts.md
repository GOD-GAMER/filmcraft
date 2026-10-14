# Shorts: vertical clips from streams

**Sequence ▸ Shorts** collects the tools for cutting vertical short-form video (YouTube Shorts,
TikTok, Instagram Reels), with clips cut from streams and VTuber VODs in mind. Every item is one
undo step and is also a command that agents can run over MCP or the control channel
(`shorts.*`, parameters below).

A typical clip:

1. Cut the moment from the VOD on a normal 16:9 sequence.
2. **Sequence ▸ Transcript ▸ Transcribe…** (once per clip; it finds every word and when it is
   said). Optionally remove filler words and pauses from the Transcript menu.
3. **Sequence ▸ Shorts ▸ Vertical Short: …** makes a 1080 x 1920 copy of the sequence and opens
   it. The 16:9 original is left as it was.
4. Optionally **Stacked Layout** for gameplay on top and the model below.
5. Put the playhead on the punchline and add **Punch In**, **Camera Shake** or **Impact Flash**.
6. **Word Captions** last, so they sit on top of everything.
7. Export with the **Social Vertical 1080×1920** preset.

## Vertical Short

| Menu item | What it does | Command |
|---|---|---|
| Crop to Fill | Scales the footage up until it covers the vertical frame; the sides are cut off. Move **Motion ▸ Position** to pan. | `shorts.verticalFill` |
| Follow Subject | As Crop to Fill, plus **Auto Reframe**, which keeps the busiest part of the picture in the middle as it moves. | `shorts.verticalFollow` |
| Blurred Background | The whole 16:9 picture in the middle, over a blurred, darkened, zoomed-in copy of itself on a new track underneath. | `shorts.verticalBlur` |

Parameters: `{"width": 1080, "height": 1920, "name": "…"}`. The footage on the lowest video track
that has footage is reframed; titles and overlays on other tracks keep their place relative to
the frame.

## Stacked Layout

**Model on Right** / **Model on Left** splits the frame: gameplay in the top pane and the
streamer's model (or camera) in the bottom pane, both cut from the same recording. The model
copies go on a new track above and are selected, so their Crop and Motion can be adjusted right
away in Effect Controls.

It works on the selected clips, or on every clip of the main picture track when nothing is
selected. Parameters (`shorts.stackRight`, `shorts.stackLeft`):

- `split`: the top pane's share of the height, 0.2 to 0.8 (default 0.5);
- `model`, `game`: the part of the recording each pane shows, as `[x0, y0, x1, y1]` fractions of
  the picture (defaults: the model is the outer 40 % of the width, from a quarter of the way
  down; the gameplay is the other side).

Each pane is filled edge to edge and never shows past the edge of the recording.

## Punch effects at the playhead

They apply to the selected clips under the playhead, or to every footage clip under it when
nothing is selected.

| Menu item | What it does | Command and parameters |
|---|---|---|
| Punch In at Playhead | Zooms in over 3 frames, holds, and eases back over 4 frames (keyframes on Motion ▸ Scale). In a stacked pane the crop zooms too, so the pane keeps its size. | `shorts.punchIn` `{"zoom": 125, "seconds": 1}` |
| Camera Shake at Playhead | A **Camera Shake** effect that hits hard at the playhead and settles to nothing. It sits before any crop, so stacked panes shake inside their edges. | `shorts.shake` `{"amount": 40, "seconds": 0.5}` |
| Impact Flash at Playhead | A full-frame colour graphic that fades out, on the first free track above. | `shorts.flash` `{"seconds": 0.3, "color": "#ffffff"}` |

All three also take `"time"` (ticks) instead of the playhead and `"clips"` instead of the
selection.

## Word Captions

Animated captions from the transcript: one graphic clip per spoken word on a new **Word
Captions** track at the top, in bold capitals with a thick outline and a shadow.

| Style | Look | Command |
|---|---|---|
| Pop | A few words at a time; the word being spoken turns yellow and grows a little. Each phrase pops in. | `shorts.captionsPop` |
| Karaoke | A few words at a time; words light up green as they are spoken. | `shorts.captionsKaraoke` |
| One Word | One big word at a time, each popping in; every third word is highlighted. | `shorts.captionsOneWord` |

A phrase ends when it is full, at a pause of half a second, at the end of a sentence and when
the speaker changes. Long phrases are shrunk to fit the frame. Each clip is an ordinary graphic,
so the font, colours and position can be changed afterwards (Graphics, Properties panel).

Parameters: `maxWords` (default 3), `maxChars` (default 16 in a vertical frame, 28 otherwise),
`color`, `highlight`, `font` (default Inter), `fontStyle` (default Bold), `size`, `position`
(`[x, y]` frame pixels; default 70 % of the way down a vertical frame), `uppercase` (default
true) and `hold` (seconds the last word of a phrase stays up, default 0.25).

## Not yet

- **Speed ramps** need time-remapping playback, which FilmCraft does not render yet.
- Sound effects are not bundled (every asset in FilmCraft must be openly licensed); import your
  own.
