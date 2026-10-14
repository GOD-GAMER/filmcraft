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
5. **Zoom on Every Cut** for jump-cut pacing, then **Find Hype Moments** to punch in on the
   loud reactions, or put the playhead on a moment and use the effects at the playhead.
6. **Bleep Swear Words** if the clip needs it, and **Duck Music Under Speech** for a music bed.
7. **Word Captions**, a **Hook Title…** and a **Progress Bar** last, so they sit on top.
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

**Track Model** (`shorts.trackModel`, `{"clips": [id]?, "wait": false}`) makes the model pane
follow the streamer: the part of the recording the pane shows is tracked frame by frame in the
background (see the Progress panel), and when it finishes the pane's Anchor Point and Crop get
smoothed keyframes that follow the movement without ever leaving the picture. It works on the
selected cropped clips, or on the model panes under the playhead.

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

Animated captions from the transcript: one graphic clip per spoken word on a new **Word
Captions** track at the top, in bold capitals with a thick outline and a shadow.

| Style | Look | Command |
|---|---|---|
| Pop | A few words at a time; the word being spoken turns yellow and grows a little. Each phrase pops in. | `shorts.captionsPop` |
| Karaoke | A few words at a time; words light up green as they are spoken. | `shorts.captionsKaraoke` |
| One Word | One big word at a time, each popping in; every third word is highlighted. | `shorts.captionsOneWord` |

A phrase ends when it is full, at a pause of half a second, at the end of a sentence and when
the speaker changes. When the transcript has more than one speaker (a collab), each speaker's
captions get their own colour (`"speakerColors": false` turns that off). Bleeped words show
masked (`F***`). Long phrases are shrunk to fit the frame. Each clip is an ordinary graphic,
so the font, colours and position can be changed afterwards (Graphics, Properties panel).

Parameters: `maxWords` (default 3), `maxChars` (default 16 in a vertical frame, 28 otherwise),
`color`, `highlight`, `font` (default Inter), `fontStyle` (default Bold), `size`, `position`
(`[x, y]` frame pixels; default 70 % of the way down a vertical frame), `uppercase` (default
true) and `hold` (seconds the last word of a phrase stays up, default 0.25).

## Sound

| Menu item | What it does | Command and parameters |
|---|---|---|
| Bleep Swear Words | Mutes each strong swear word in the transcript (Volume keyframes on its clip), puts a beep over it on a new **Bleeps** track, and masks it in Word Captions (`F***`), before or after the captions are made. | `shorts.bleep` `{"words": [...]?, "extraWords": [...]?, "hz": 1000, "beepDb": -14}` |
| Duck Music Under Speech | Select the music; it is tagged Music, the clips under it Dialogue, and Essential Sound's ducking lowers the music whenever someone talks. | `shorts.duckMusic` `{"reduceDb": -15}` |
| Sound Effect at Playhead | Drops the sound selected in the Project panel at the playhead on the first free audio track (one is added if needed), without moving anything. Import your sound effects first. | `shorts.soundEffect` `{"item": id?}` |

`words` replaces the built-in list (common strong swears; mild words such as "damn" are left
alone); `extraWords` adds to it.

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
