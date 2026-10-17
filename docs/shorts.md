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
4. **Layout…** for the gameplay and the model (drag their areas over a frame of the stream).
5. **Zoom on Every Cut** for jump-cut pacing, then **Find Hype Moments** to punch in on the
   loud reactions, or put the playhead on a moment and use the effects at the playhead.
6. **Bleep Swear Words** if the clip needs it, and **Duck Music Under Speech** for a music bed.
7. **Word Captions…**, a **Hook Title…** and a **Progress Bar** last, so they sit on top.
8. Export with the **Social Vertical 1080×1920** preset.

## Vertical Short

| Menu item | What it does | Command |
|---|---|---|
| Crop to Fill | Scales the footage up until it covers the vertical frame; the sides are cut off. Move **Motion ▸ Position** to pan. | `shorts.verticalFill` |
| Follow Subject | As Crop to Fill, plus **Auto Reframe**, which keeps the busiest part of the picture in the middle as it moves. | `shorts.verticalFollow` |
| Blurred Background | The whole 16:9 picture in the middle, over a blurred, darkened, zoomed-in copy of itself on a new track underneath. | `shorts.verticalBlur` |

Parameters: `{"width": 1080, "height": 1920, "name": "…"}`. The footage on the lowest video track
that has footage is reframed; titles and overlays on other tracks keep their place relative to
the frame.

## Layout

**Layout…** arranges a widescreen stream recording in the vertical frame and opens a window to
adjust it; the Program monitor shows every change as you make it (OK keeps it, Cancel undoes it,
and the whole session in the window is one undo step).

- **Presets:** **Stack** (the default) keeps all of the gameplay beside the model at full width,
  with the model below in a box of its own shape (about a third of the frame tall); **Split**
  gives the gameplay the top 60 % and the model the bottom 40 % (full width); **Picture in
  picture** makes the gameplay larger with the model in a bordered box over its lower corner.
  **Model in the stream** says which side of the recording the model is on.
- **Picker:** a frame of the recording with the **Gameplay** (green) and **Model** (pink) areas.
  Drag a box to move it, or its edges to resize it. Trim the gameplay box's bottom edge to hide
  captions or overlays the stream burned in.
- **Gameplay:** *Width kept* (how much of the gameplay's width stays; less zooms in and makes the
  pane taller), *Pan*, *Down* (its place on screen), *Corner roundness*.
- **Model:** *Box width*, *Box height*, *Across*, *Down*, *Corner roundness*, *Border* and its
  colour.
- **Background:** a blurred, darkened copy of the recording behind both, or black.

Applying a layout again replaces the previous one (its model and background copies). The model
copies go on a **Model** track above the gameplay, the blurred copy on **Layout Background**
below it. Both panes use **Rounded Crop**, so they can also be adjusted in Effect Controls.

**Stacked Layout: Model on Right / Left** apply the Split preset straight away.

| Command | Parameters |
|---|---|
| `shorts.layout`, `shorts.stackRight`, `shorts.stackLeft` | `{"preset": "stack\|split\|pip", "side": "right\|left", "game": [x0, y0, x1, y1], "gameWidth": 0.3–1, "gamePan": −1–1, "gameY": 0–1, "model": [x0, y0, x1, y1], "modelWidth", "modelHeight", "modelX", "modelY" (fractions of the frame), "corner", "gameCorner", "border" (pixels at 1080 wide), "borderColor", "background": "blur\|black", "live": bool}` |

Regions are fractions of the recording (`[0.70, 0.36, 1, 1]` is the right 30 % from a little
over a third of the way down, where many streams put the model, under a chat window).

**Track Model** (`shorts.trackModel`, `{"clips": [id]?, "wait": false}`) makes the model pane
follow a streamer who moves around (a camera): the part of the recording the pane shows is
tracked frame by frame in the background (see the Progress panel), and when it finishes the
pane's Anchor Point and Crop get smoothed keyframes that follow the movement without ever
leaving the picture. A VTuber model stays in one place in the stream layout, so it does not need
tracking; tracking it follows the gameplay moving behind the model instead.

## Effects at the playhead

They apply to the selected clips under the playhead, or to every footage clip under it when
nothing is selected.

| Menu item | What it does | Command and parameters |
|---|---|---|
| Punch In at Playhead | Zooms in over 3 frames, holds, and eases back over 4 frames (keyframes on Motion ▸ Scale). In a stacked pane the crop zooms too, so the pane keeps its size. | `shorts.punchIn` `{"zoom": 125, "seconds": 1}` |
| Camera Shake at Playhead | A **Camera Shake** effect that hits hard at the playhead and settles to nothing. It sits before any crop, so stacked panes shake inside their edges. | `shorts.shake` `{"amount": 40, "seconds": 0.5}` |
| Speed Ramp at Playhead | The next second of the clip ramps down to slow motion and back up (Time Remapping), so the moment lasts longer. Later clips move right by the extra time; the moment's sound plays at normal speed and then pauses, so everything after it stays in sync. Stacked panes ramp together. | `shorts.speedRamp` `{"speed": 35, "seconds": 1, "ramp": 0.2}` |
| Glitch Hit at Playhead | Stuttering **RGB Split** and **Wave Warp** with a small shake, before any crop. | `shorts.glitch` `{"amount": 40, "seconds": 0.35}` |
| Impact Flash at Playhead | A full-frame colour graphic that fades out, on the first free track above. | `shorts.flash` `{"seconds": 0.3, "color": "#ffffff"}` |
| Freeze Frame at Playhead | Inserts a held frame (later clips move right; the sound pauses), black and white and slowly pushing in, with a flash. A stacked layout's model pane holds too. | `shorts.freezeFrame` `{"seconds": 1.5, "zoom": 115, "flash": true}` |

They also take `"time"` (ticks) instead of the playhead and `"clips"` instead of the selection.

## Automatic effects

| Menu item | What it does | Command and parameters |
|---|---|---|
| Find Hype Moments | Listens to the sequence's mix (or its In/Out range) for moments much louder than the talking around them: screams, laughs, hype. Each one gets an orange **Hype** marker, a punch-in and a shake. | `shorts.hype` `{"effects": ["punch", "shake", "glitch", "flash"], "sensitivity": 9, "maxMoments": 8, "minGap": 2, "markersOnly": false}` |
| Find Chat Highlights… | Asks for the stream's chat replay and adds a pink **Chat Spike** marker wherever chat suddenly got busy, with the message count and the most repeated words. | `shorts.chatHighlights` `{"path": "…", "offset": 0, "window": 5, "maxMoments": 10}` |
| Effects on the Beat | Finds the beat of the selected music clip and adds a small zoom pulse (or flash, shake…) to the footage on every second beat. | `shorts.beatSync` `{"effects": ["pulse"], "every": 2, "amount": 105}` |
| Zoom on Every Cut | Zooms every other clip of the main picture track in (static), the jump-cut look. Make the cuts first (Transcript ▸ Remove Pauses). Stacked panes zoom inside their crop. | `shorts.zoomCuts` `{"zoom": 112}` |
| Effects on Words… | Asks for words or short phrases (comma-separated) and an effect, then adds it wherever they are said. Needs a transcript. | `shorts.keywordEffects` `{"words": "bonk, oh my god", "effect": "punch"}` (or a list of effects) |

`sensitivity` is how many dB above the surrounding audio a moment must be: lower finds more.

## Graphics

| Menu item | What it does | Command and parameters |
|---|---|---|
| Hook Title… | Big text that slams in at the start, wobbles and fades (default "WAIT FOR IT…"), on its own track. Edit it afterwards like any graphic. | `shorts.hookTitle` `{"text": "…", "seconds": 2.5, "color": "#ffd60a"}` |
| Chat Overlay… | Asks for the stream's chat replay and shows it in a box on its own **Chat** track: the newest messages, names in their chat colours. | `shorts.chatOverlay` `{"path": "…", "offset": 0, "lines": 6, "maxChars": 34, "position": "left"}` |
| Progress Bar | A bar along the bottom (or top) that fills over the whole sequence, over a faint track. | `shorts.progressBar` `{"position": "bottom", "color": "#ffd60a", "height": 16}` |

## Word Captions

Animated captions from the transcript: one graphic clip per spoken word on a **Word Captions**
track at the top. **Word Captions…** opens a window with every option and previews each change
in the Program monitor (OK keeps it, Cancel undoes it). Applying again replaces the previous
Word Captions. The quick items (Pop, Karaoke, Highlight Box, One Word) apply a style straight
away with its defaults.

| Style | Look |
|---|---|
| Pop | A few words at a time; the word being spoken changes colour and grows a little. |
| Karaoke | Words light up as they are spoken. |
| Highlight box | A coloured box sits behind the word being spoken. |
| One word | One big word at a time, each bouncing in. |
| Plain | Phrases only, no highlight. |

Options:

- **Text:** font and style (any font installed on the computer), size, words at once, letters per
  line, lines (1–3), capitals, punctuation (off by default), colour per speaker (off by default:
  speaker detection can mistake one voice, singing especially, for several), and **Fix words**
  (`nero=Neuro, kot=cot`) for words the transcription got wrong.
- **Colours:** text, highlight (the box's colour in Highlight box), highlighted text, how much
  bigger the spoken word is, outline width and colour, shadow, a box behind the whole phrase
  (colour, opacity) and the boxes' roundness.
- **Placement and motion:** across and down (by default just over the bottom edge of a layout's
  gameplay pane, or 70 % down), how phrases appear (pop, bounce, fade, slide up, none) and how
  long they stay after the speech.

A phrase ends when it is full, at a pause of half a second, at the end of a sentence and when
the speaker changes. Phrases too wide for the frame shrink to fit. Bleeped words show masked
(`F***`). Each clip is an ordinary graphic, so it can be changed by hand afterwards too (a later
Word Captions… replaces those changes).

Command: `shorts.captions` (and `shorts.captionsPop`, `shorts.captionsKaraoke`,
`shorts.captionsBox`, `shorts.captionsOneWord`) with `{"style", "maxWords", "maxChars", "lines",
"font", "fontStyle", "size" (pixels at 1080 wide), "uppercase", "punctuation", "color",
"highlight", "highlightText", "grow", "outline", "outlineColor", "shadow", "background",
"backgroundColor", "backgroundOpacity", "radius", "x", "y", "animation", "hold",
"speakerColors", "fixes", "replace", "live"}`.

## Sound

| Menu item | What it does | Command and parameters |
|---|---|---|
| Bleep Swear Words | Mutes each strong swear word in the transcript (Volume keyframes on its clip), puts a beep over it on a new **Bleeps** track, and masks it in Word Captions (`F***`), before or after the captions are made. | `shorts.bleep` `{"words": [...]?, "extraWords": [...]?, "hz": 1000, "beepDb": -14}` |
| Duck Music Under Speech | Select the music; it is tagged Music, the clips under it Dialogue, and Essential Sound's ducking lowers the music whenever someone talks. | `shorts.duckMusic` `{"reduceDb": -15}` |
| Sound Effect at Playhead | Drops the sound selected in the Project panel at the playhead on the first free audio track (one is added if needed), without moving anything. Import your sound effects first. | `shorts.soundEffect` `{"item": id?}` |

`words` replaces the built-in list (common strong swears; mild words such as "damn" are left
alone); `extraWords` adds to it. Words the transcription already starred out (`f***`) count
too. Each beep is padded by `padding` seconds (0.06) either side, since word timings from speech
recognition can be a little early or late, and is named after the words it covers ("Bleep:
shit").

Word Captions, Bleep Swear Words and Effects on Words need a transcript, which needs a build
with speech recognition (the `whisper` feature; the fork's Windows test build has it).

## Chat replays

Chat Overlay… and Find Chat Highlights… read the chat downloads most clippers already use:
**TwitchDownloader** chat JSON, **chat-downloader** JSON, and **yt-dlp**'s YouTube
`.live_chat.json`. Chat times count from the start of the stream recording, and reach the
timeline through the clips of that recording (the selected one, or the footage under the
playhead), so the chat lines up with every cut. Use `offset` (seconds) when the recording on the
timeline does not start where the stream did.

## Time Remapping

Clips now play their **Time Remapping ▸ Speed** keyframes (Effect Controls): the picture speeds
up and slows down with them, and timeline thumbnails follow. The sound is not remapped.

## Not yet

- Sound effects are not bundled (every asset in FilmCraft must be openly licensed); import your
  own and use **Sound Effect at Playhead**.
