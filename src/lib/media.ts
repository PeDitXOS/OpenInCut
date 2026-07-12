/** The one list of importable media extensions: the native-drop filter and
 *  the file picker must accept exactly the same files. */
export const MEDIA_EXTENSIONS = [
  "mp4", "mov", "mkv", "webm", "avi", "m4v", "mts", "mpg",
  "wav", "mp3", "m4a", "aac", "flac", "ogg", "aiff",
  "png", "jpg", "jpeg", "webp", "bmp", "tiff", "gif",
] as const;
