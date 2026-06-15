// Bundled web fonts (no CDN — local-first / data-sovereignty). These are the exact
// families + weights the prototype requests from Google Fonts, shipped via
// @fontsource so the build inlines the woff2 files. Family names match the
// tokens.css --fd/--fb/--fu/--fm stacks, so no other change is needed.
//
// Playfair Display — display / serif headers (500, 600, + italics 400/500/600)
import "@fontsource/playfair-display/latin-500.css";
import "@fontsource/playfair-display/latin-600.css";
import "@fontsource/playfair-display/latin-400-italic.css";
import "@fontsource/playfair-display/latin-500-italic.css";
import "@fontsource/playfair-display/latin-600-italic.css";

// Lora — body / email content (400, 500, + italics 400/500)
import "@fontsource/lora/latin-400.css";
import "@fontsource/lora/latin-500.css";
import "@fontsource/lora/latin-400-italic.css";
import "@fontsource/lora/latin-500-italic.css";

// DM Sans — uppercase UI labels, nav (300, 400, 500, 600)
import "@fontsource/dm-sans/latin-300.css";
import "@fontsource/dm-sans/latin-400.css";
import "@fontsource/dm-sans/latin-500.css";
import "@fontsource/dm-sans/latin-600.css";

// DM Mono — numbers, timestamps, mono data (300, 400, 500)
import "@fontsource/dm-mono/latin-300.css";
import "@fontsource/dm-mono/latin-400.css";
import "@fontsource/dm-mono/latin-500.css";
