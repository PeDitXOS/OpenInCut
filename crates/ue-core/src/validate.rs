//! Validation of the project invariants (PLAN section 4.2).

use std::collections::HashSet;

use crate::model::*;

/// Returns the list of violations (empty = valid project).
pub fn validate(project: &Project) -> Vec<String> {
    let mut issues = vec![];

    // Globally unique ids.
    let mut seen: HashSet<String> = HashSet::new();
    let mut check_id = |id: Id, what: &str, issues: &mut Vec<String>| {
        if !seen.insert(id.to_string()) {
            issues.push(format!("duplicate id in {what}: {id}"));
        }
    };
    check_id(project.id, "project", &mut issues);
    for a in &project.assets {
        check_id(a.id, "asset", &mut issues);
    }
    for t in &project.transcripts {
        check_id(t.id, "transcript", &mut issues);
    }
    for s in &project.sequences {
        check_id(s.id, "sequence", &mut issues);
        for tr in &s.tracks {
            check_id(tr.id, "track", &mut issues);
            for c in &tr.clips {
                check_id(c.id, "clip", &mut issues);
            }
        }
    }

    if project.sequence(project.active_sequence).is_none() {
        issues.push("active_sequence does not exist".into());
    }

    for seq in &project.sequences {
        if seq.fps.0 == 0 || seq.fps.1 == 0 {
            issues.push(format!("sequence {}: invalid fps", seq.name));
        }
        for track in &seq.tracks {
            // Order and non-overlap.
            for w in track.clips.windows(2) {
                if w[0].start > w[1].start {
                    issues.push(format!("track {}: clips out of order", track.name));
                }
                if w[0].end() > w[1].start {
                    issues.push(format!(
                        "track {}: overlap between {} and {}",
                        track.name, w[0].id, w[1].id
                    ));
                }
            }
            for clip in &track.clips {
                if clip.duration <= 0 {
                    issues.push(format!("clip {}: duration <= 0", clip.id));
                }
                if clip.speed <= 0.0 {
                    issues.push(format!("clip {}: speed <= 0", clip.id));
                }
                if let ClipPayload::Media { asset_id, src_in, src_out } = &clip.payload {
                    if src_in >= src_out {
                        issues.push(format!("clip {}: src_in >= src_out", clip.id));
                    }
                    if *src_in < 0 {
                        issues.push(format!("clip {}: src_in < 0", clip.id));
                    }
                    match project.asset(*asset_id) {
                        None => issues.push(format!("clip {}: asset {asset_id} does not exist", clip.id)),
                        Some(a) => {
                            if *src_out > a.probe.duration_us && a.probe.duration_us > 0 {
                                issues.push(format!(
                                    "clip {}: src_out {} > asset duration {}",
                                    clip.id, src_out, a.probe.duration_us
                                ));
                            }
                        }
                    }
                }
                // Curves: strictly increasing keys.
                let mut check_param = |p: &crate::keyframe::Param, what: &str| {
                    if let crate::keyframe::Param::Curve(c) = p {
                        if c.keys.is_empty() {
                            issues.push(format!("clip {}: empty curve in {what}", clip.id));
                        }
                        for w in c.keys.windows(2) {
                            if w[0].t >= w[1].t {
                                issues.push(format!(
                                    "clip {}: non-increasing keys in {what}",
                                    clip.id
                                ));
                            }
                        }
                    }
                };
                check_param(&clip.transform.opacity, "opacity");
                check_param(&clip.transform.rotation, "rotation");
                check_param(&clip.transform.position.0, "position.x");
                check_param(&clip.transform.position.1, "position.y");
                check_param(&clip.transform.scale.0, "scale.x");
                check_param(&clip.transform.scale.1, "scale.y");
                check_param(&clip.audio.gain_db, "gain_db");
                check_param(&clip.audio.pan, "pan");
                for e in &clip.effects {
                    for (k, p) in &e.params {
                        check_param(p, &format!("effect {}.{}", e.effect_id, k));
                    }
                }
            }
        }
    }

    issues
}
