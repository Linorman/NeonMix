//! Page bodies. Each page leads with one hero that answers its main question
//! (is the room sharing? am I sending? what is playing?), then setup steps,
//! then details in collapsible panels.
use super::*;

mod about;
mod airplay;
mod devices;
mod diagnostics;
mod mixer;
mod room;
mod sender;

pub(crate) const TWO_COLUMNS: f32 = 760.0;

pub(crate) fn role_name(role: Role) -> &'static str {
    match role {
        Role::Admin => "管理员",
        Role::Controller => "控制者",
        Role::Member => "成员",
    }
}

pub(crate) fn role_color(role: Role) -> egui::Color32 {
    match role {
        Role::Admin => theme::ACCENT,
        Role::Controller => theme::SUCCESS,
        Role::Member => theme::TEXT_2,
    }
}

pub(crate) fn airplay_source_matches(source: &Value, needle: &str) -> bool {
    needle.is_empty()
        || source["source_name"]
            .as_str()
            .unwrap_or("AirPlay 来源")
            .to_lowercase()
            .contains(needle)
        || source["last_name"]
            .as_str()
            .is_some_and(|name| name.to_lowercase().contains(needle))
        || source["source_id"]
            .as_str()
            .is_some_and(|id| id.to_lowercase().contains(needle))
        || "airplay".contains(needle)
}

pub(crate) fn device_state(
    device: &neonmix_control::Device,
    streams: usize,
) -> (&'static str, widgets::Tone) {
    use widgets::Tone;
    if device.revoked {
        ("已撤销", Tone::Danger)
    } else if !device.playback_allowed {
        ("已断开", Tone::Warning)
    } else if streams > 0 {
        ("发送中", Tone::Success)
    } else {
        ("空闲", Tone::Neutral)
    }
}

pub(crate) fn short_id(id: u64) -> String {
    let s = id.to_string();
    s[s.len().saturating_sub(6)..].to_owned()
}

/// Human text for a diagnostics value: strings without JSON quotes,
/// integers with digit grouping, missing values named explicitly.
pub(crate) fn value_text(value: &Value) -> String {
    match value {
        Value::Null => "未取得".into(),
        Value::Bool(b) => if *b { "是" } else { "否" }.into(),
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_u64() {
            Some(v) => group_digits(v),
            None => n.to_string(),
        },
        other => other.to_string(),
    }
}

pub(crate) fn group_digits(v: u64) -> String {
    let digits = v.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Desktop {
    /// Fader bound to a gain draft. The draft follows the hand during a drag
    /// and stays until the authoritative snapshot reports the value (failed
    /// commands clear all drafts), so the handle never snaps back. Returns
    /// the value to commit.
    pub(crate) fn gain_control(
        &mut self,
        ui: &mut egui::Ui,
        key: u64,
        current: f32,
        label: &str,
        size: widgets::FaderSize,
    ) -> (Option<f32>, egui::Response) {
        let mut gain = self.gain_drafts.get(&key).copied().unwrap_or(current);
        let fader = widgets::fader(ui, key, &mut gain, label, size);
        if fader.response.changed() {
            self.gain_drafts.insert(key, gain);
        }
        if fader.committed {
            self.gain_drafts.insert(key, gain);
            let commit = ((gain - current).abs() > 0.01).then_some(gain);
            return (commit, fader.response);
        }
        if !fader.response.dragged()
            && self
                .gain_drafts
                .get(&key)
                .is_some_and(|d| (d - current).abs() < 0.01)
        {
            self.gain_drafts.remove(&key);
        }
        (None, fader.response)
    }

    pub(crate) fn connection_override(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(
            RichText::new("连接地址（故障排查）")
                .size(theme::SMALL + 0.5)
                .color(theme::TEXT_2),
        )
        .id_salt("connection-override")
        .show(ui, |ui| {
            let before = self.hub_address.clone();
            widgets::field(ui, "HTTPS 地址（可留空）", &mut self.hub_address, false);
            if before != self.hub_address {
                self.fresh = None;
                self.synced = None;
                self.last_poll = Instant::now() - Duration::from_secs(5);
            }
            widgets::note(
                ui,
                "留空使用自动发现；任何地址都必须匹配已配对的证书和房间身份。",
            );
        });
    }
}
