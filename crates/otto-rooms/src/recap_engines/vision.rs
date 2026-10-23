use otto_core::{Error, Result};

/// Validate JPEG dimensions before the native decoder allocates its image.
pub fn jpeg_dimensions(bytes: &[u8]) -> Result<(u16, u16)> {
    let invalid =
        || Error::Invalid("Screen sample must be a JPEG up to 1280×720 pixels and 256 KiB".into());
    if bytes.len() > 262144 || !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(invalid());
    }
    let mut at = 2;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xff {
            return Err(invalid());
        }
        while at < bytes.len() && bytes[at] == 0xff {
            at += 1;
        }
        if at + 3 > bytes.len() {
            return Err(invalid());
        }
        let marker = bytes[at];
        at += 1;
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let size = u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize;
        if size < 2 || at + size > bytes.len() {
            return Err(invalid());
        }
        if matches!(marker, 0xc0..=0xc3) {
            if size < 8 {
                return Err(invalid());
            }
            let height = u16::from_be_bytes([bytes[at + 3], bytes[at + 4]]);
            let width = u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]);
            if width == 0
                || height == 0
                || width > 1280
                || height > 1280
                || u32::from(width) * u32::from(height) > 1280 * 720
            {
                return Err(invalid());
            }
            return Ok((width, height));
        }
        at += size;
    }
    Err(invalid())
}
pub async fn recognize_screen(image: &[u8]) -> Result<String> {
    jpeg_dimensions(image)?;
    #[cfg(target_os = "macos")]
    {
        let executable = std::env::current_exe().map_err(|e| Error::Internal(e.to_string()))?;
        let dir = tempfile::tempdir().map_err(|e| Error::Internal(e.to_string()))?;
        let out = super::process::run(
            executable
                .to_str()
                .ok_or_else(|| Error::Internal("Invalid daemon executable path".into()))?,
            &["room-ocr".into()],
            image,
            dir.path(),
            std::time::Duration::from_secs(30),
        )
        .await?;
        serde_json::from_slice(&out)
            .map_err(|_| Error::Upstream("Screen recognition returned invalid output".into()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(Error::Invalid(
            "Local screen-text recognition requires macOS".into(),
        ))
    }
}
/// Internal ottod child mode. It must run before daemon configuration, logging,
/// database access or listeners. A separate process makes native Vision fully
/// cancellable; dropping a blocking Rust task cannot stop an in-flight request.
pub fn run_ocr_stdio() -> bool {
    use std::io::Read;
    #[cfg(target_os = "macos")]
    use std::io::Write;
    let mut bytes = Vec::new();
    if std::io::stdin()
        .take(262145)
        .read_to_end(&mut bytes)
        .is_err()
        || jpeg_dimensions(&bytes).is_err()
    {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        match recognize(&bytes)
            .and_then(|text| serde_json::to_vec(&text).map_err(|e| Error::Internal(e.to_string())))
        {
            Ok(out) => std::io::stdout().write_all(&out).is_ok(),
            Err(_) => false,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
#[cfg(target_os = "macos")]
fn recognize(bytes: &[u8]) -> Result<String> {
    use objc2::AnyThread;
    use objc2_foundation::{NSArray, NSData, NSDictionary};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };
    objc2::rc::autoreleasepool(|_| {
        let data = NSData::with_bytes(bytes);
        let handler = VNImageRequestHandler::initWithData_options(
            VNImageRequestHandler::alloc(),
            &data,
            &NSDictionary::new(),
        );
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(false);
        // macOS12-compatible. Newer language-auto-detection selectors are not
        // used; screenshots are preserved alongside this best-effort OCR.
        unsafe {
            request.setPreferBackgroundProcessing(true);
        }
        let requests = NSArray::from_slice(&[request.as_ref() as &VNRequest]);
        handler.performRequests_error(&requests).map_err(|_| {
            Error::Upstream("Could not recognize text in this screen sample".into())
        })?;
        let mut text = String::new();
        if let Some(observations) = request.results() {
            for index in 0..observations.count() {
                let observation = observations.objectAtIndex(index);
                let candidates = observation.topCandidates(1);
                if let Some(candidate) = candidates.firstObject() {
                    let line = candidate.string().to_string();
                    if text.len() + line.len() + 1 > 32768 {
                        break;
                    }
                    text.push_str(&line);
                    text.push('\n');
                }
            }
        }
        Ok(text)
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_and_large_jpeg_cannot_reach_native_decoder() {
        assert!(jpeg_dimensions(b"hello").is_err());
        assert!(jpeg_dimensions(&[0xff, 0xd8, 0xff, 0xe0, 0xff, 0xff]).is_err());
        let mut image = vec![0xff, 0xd8, 0xff, 0xc0, 0, 8, 8, 2, 208, 5, 0, 0];
        assert_eq!(jpeg_dimensions(&image).unwrap(), (1280, 720));
        image[9] = 6;
        assert!(jpeg_dimensions(&image).is_err());
        image.resize(262145, 0);
        assert!(jpeg_dimensions(&image).is_err());
    }
}
