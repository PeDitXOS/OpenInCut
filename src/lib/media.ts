/** The one list of importable media extensions: the native-drop filter and
 *  the file picker must accept exactly the same files. */
export const MEDIA_EXTENSIONS = [
  "mp4", "mov", "mkv", "webm", "avi", "m4v", "mts", "ts", "m2ts", "mpg",
  "wav", "mp3", "m4a", "aac", "flac", "ogg", "aiff",
  "png", "jpg", "jpeg", "webp", "bmp", "tiff", "gif",
  "fcpxml", "fcpxmld",
] as const;

/** FCP XML project file extensions (not media — handled separately). */
export const FCP_EXTENSIONS: ReadonlySet<string> = new Set(["fcpxml", "fcpxmld"]);
