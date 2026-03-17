//! OCR text recognition via macOS Vision.framework.
//!
//! Calls VNRecognizeTextRequest through a small ObjC bridge via osascript.
//! Supports Chinese, English, Japanese, Korean — fully offline.

/// Recognized text block.
#[derive(Clone, serde::Serialize)]
pub struct OcrTextBlock {
    pub text: String,
    pub confidence: f64,
}

/// Result of OCR recognition.
#[derive(Clone, serde::Serialize)]
pub struct OcrResult {
    pub full_text: String,
    pub blocks: Vec<OcrTextBlock>,
}

/// Perform OCR on an image file using Vision.framework via JXA (JavaScript for Automation).
/// JXA handles ObjC bridging more reliably than pure AppleScript for framework calls.
pub fn recognize_text(image_path: &str) -> Result<OcrResult, String> {
    // Sanitize path to prevent JXA injection via single quotes
    let safe_path = image_path.replace('\\', "\\\\").replace('\'', "\\'");
    let script = format!(
        r#"ObjC.import('Vision');
ObjC.import('AppKit');

var url = $.NSURL.fileURLWithPath('{safe_path}');
var image = $.NSImage.alloc.initWithContentsOfURL(url);
if (!image || image.isNil()) {{
    JSON.stringify({{error: "Failed to load image"}});
}}

var handler = $.VNImageRequestHandler.alloc.initWithDataOptions(
    image.TIFFRepresentation, $()
);

var request = $.VNRecognizeTextRequest.alloc.init;
request.recognitionLevel = $.VNRequestTextRecognitionLevelAccurate;
request.usesLanguageCorrection = true;

// Set recognition languages
var langs = $.NSArray.arrayWithArray([
    'zh-Hans', 'zh-Hant', 'en-US', 'ja', 'ko'
]);
request.recognitionLanguages = langs;

var success = handler.performRequestsError($.NSArray.arrayWithObject(request), $());

var results = request.results;
var blocks = [];
for (var i = 0; i < results.count; i++) {{
    var obs = results.objectAtIndex(i);
    var candidates = obs.topCandidates(1);
    if (candidates.count > 0) {{
        var candidate = candidates.objectAtIndex(0);
        var text = candidate.string.js;
        var conf = candidate.confidence;
        blocks.push({{text: text, confidence: conf}});
    }}
}}

JSON.stringify({{blocks: blocks}});"#
    );

    let output = std::process::Command::new("osascript")
        .args(["-l", "JavaScript", "-e", &script])
        .output()
        .map_err(|e| format!("Failed to run osascript: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.is_empty() {
            return Ok(OcrResult {
                full_text: String::new(),
                blocks: vec![],
            });
        }
        return Err(format!("Vision OCR failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Parse JSON output
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("Failed to parse OCR output: {} (raw: {})", e, stdout))?;

    if let Some(err) = parsed.get("error") {
        return Err(err.as_str().unwrap_or("Unknown error").to_string());
    }

    let mut blocks = Vec::new();
    let mut full_text_lines = Vec::new();

    if let Some(arr) = parsed.get("blocks").and_then(|b| b.as_array()) {
        for item in arr {
            let text = item.get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let confidence = item.get("confidence")
                .and_then(|c| c.as_f64())
                .unwrap_or(1.0);

            if !text.is_empty() {
                full_text_lines.push(text.clone());
                blocks.push(OcrTextBlock { text, confidence });
            }
        }
    }

    let full_text = full_text_lines.join("\n");

    println!(
        "[ocr] Recognized {} blocks, {} chars from {}",
        blocks.len(),
        full_text.len(),
        image_path
    );

    Ok(OcrResult { full_text, blocks })
}
