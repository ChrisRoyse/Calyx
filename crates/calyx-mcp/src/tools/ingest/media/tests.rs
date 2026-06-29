use super::*;

#[test]
fn unsupported_video_extension_fails_closed() {
    let err = media_extension(Path::new("clip.txt"), Modality::Video).unwrap_err();
    assert!(format!("{err:?}").contains("CALYX_MEDIA_UNSUPPORTED_EXTENSION"));
}

#[test]
fn wav_magic_is_checked_before_decode() {
    let err = validate_magic(b"not-wave", Modality::Audio, "wav").unwrap_err();
    assert!(format!("{err:?}").contains("CALYX_MEDIA_MAGIC_MISMATCH"));
}
