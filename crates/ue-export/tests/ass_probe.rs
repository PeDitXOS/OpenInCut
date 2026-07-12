//! Regression: with NO explicit highlight_color, the export's karaoke must
//! light words in the amber accent — the same default the drawtext preview
//! and the play compositor use. It once fell back to the text colour, which
//! made the exported karaoke white-on-white (invisible).

use ue_core::model::*;

#[test]
fn karaoke_style_primary_is_the_amber_accent() {
    let mut project = Project::new("probe");
    let seq_id = project.active_sequence;
    let aid = Id::new();
    let doc_id = Id::new();
    project.transcripts.push(TranscriptDoc {
        id: doc_id,
        asset_id: aid,
        language: "es".into(),
        model: "t".into(),
        words: vec![Word {
            text: "ONE".into(),
            start_us: 300_000,
            end_us: 800_000,
            confidence: 1.0,
            rejected: false,
            display: None,
        }],
        segments: vec![Segment {
            text: "ONE".into(),
            start_us: 200_000,
            end_us: 900_000,
            word_range: (0, 1),
            emotion: None,
            volume_rms: 0.0,
        }],
        global_avg_volume: 0.0,
    });
    let seq = project.sequence_mut(seq_id).unwrap();
    let v1 = seq.tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().id;
    // a media clip of the asset so word times map onto the timeline
    let media = Clip::new_media(aid, 0, 3_000_000, 0);
    let clip = Clip {
        id: Id::new(),
        payload: ClipPayload::Subtitles {
            transcript_id: doc_id,
            style: TextStyle::default(),
            mode: SubtitleMode::Karaoke,
            max_words: None,
        },
        start: 0,
        duration: 3_000_000,
        speed: 1.0,
        effects: vec![],
        transform: Default::default(),
        audio: Default::default(),
        transition_in: None,
        transition_out: None,
        label_color: None,
        name: None,
        group: None,
    };
    let track = project.sequence_mut(seq_id).unwrap().tracks.iter_mut().find(|t| t.id == v1).unwrap();
    track.clips.push(media);
    track.clips.push(clip);

    let seq = project.sequence(seq_id).unwrap();
    let script = ue_export::ass::build_script(&project, seq, 1920, 1080, None)
        .expect("karaoke subtitles produce a script");
    let style_line = script
        .lines()
        .find(|l| l.starts_with("Style: s0"))
        .expect("style s0 present");
    assert!(
        style_line.to_uppercase().contains("24B2FF"),
        "karaoke PrimaryColour must be the amber accent (&H0024B2FF): {style_line}"
    );
}
