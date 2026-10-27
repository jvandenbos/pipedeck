.pragma library
// One set of column widths for the block headers and their rows, so the
// captions sit over the controls they name (SPEC-UI §4.2). Pixels.
var pad = 10        // a row's inner left/right padding
var spacing = 4     // between controls in a row
var lead = 20       // the row's glyph or app icon
var mute = 28
var slider = 112
var percent = 42
var boost = 30
// The device / redirect popup button, with its warning: wide enough for
// "Digital Microphone" at 13 px in Adwaita Sans (116 px) beside the glyph and
// the up/down mark (52 px of chrome), unelided.
var device = 172
var fx = 26
var rowHeight = 36
// "Volume" sits over the slider and percent; the mute cell before them is
// left to the title.
var volume = slider + percent + spacing
// Applications have no FX column; their redirect button takes its width.
var redirect = device + spacing + fx
