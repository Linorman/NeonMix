//! Page bodies. Each page leads with one hero that answers its main question
//! (is the room sharing? am I sending? what is playing?), then setup steps,
//! then details in collapsible panels.
use super::*;

mod about;
mod airplay;
mod console;
mod devices;
mod diagnostics;
mod live;
mod mixer;
mod room;
mod sender;

pub(crate) const TWO_COLUMNS: f32 = 760.0;

pub(crate) fn role_name(role: Role) -> Message {
    match role {
        Role::Admin => Message::RoleAdmin,
        Role::Controller => Message::RoleController,
        Role::Member => Message::RoleMember,
    }
}

pub(crate) fn role_color(role: Role) -> egui::Color32 {
    match role {
        Role::Admin => theme::accent(),
        Role::Controller => theme::success(),
        Role::Member => theme::text_2(),
    }
}

pub(crate) fn airplay_source_matches(source: &Value, needle: &str) -> bool {
    needle.is_empty()
        || source["source_name"]
            .as_str()
            .unwrap_or("")
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
) -> (Message, widgets::Tone) {
    use widgets::Tone;
    if device.revoked {
        (Message::DeviceRevoked, Tone::Danger)
    } else if !device.playback_allowed {
        (Message::DeviceDisconnected, Tone::Warning)
    } else if streams > 0 {
        (Message::DeviceSending, Tone::Success)
    } else {
        (Message::DeviceIdle, Tone::Neutral)
    }
}

/// Prefer structured process faults; legacy machine codes remain a bounded fallback.
pub(crate) fn process_error(process: &neonmix_desktop_service::ProcessStatus) -> Message {
    let fault = process.fault.clone().or_else(|| {
        process
            .error
            .as_deref()
            .and_then(neonmix_desktop_service::Fault::from_machine_code)
    });
    user_error(UiError { fault })
}

pub(crate) fn short_id(id: u64) -> String {
    let s = id.to_string();
    s[s.len().saturating_sub(6)..].to_owned()
}

/// Human text for a diagnostics value: strings without JSON quotes,
/// integers with digit grouping, missing values named explicitly.
impl Desktop {
    pub(crate) fn value_text(&self, value: &Value) -> String {
        match value {
            Value::Null => self.tr(&Message::CommonUnavailable),
            Value::Bool(true) => self.tr(&Message::CommonYes),
            Value::Bool(false) => self.tr(&Message::CommonNo),
            Value::String(s) => s.clone(),
            Value::Number(n) => match n.as_u64() {
                Some(v) => group_digits(v),
                None => n.to_string(),
            },
            other => other.to_string(),
        }
    }
}

pub(crate) fn group_digits(v: u64) -> String {
    // First-release technical readouts share the documented grouping policy.
    neonmix_i18n::LocaleFormat::new(neonmix_i18n::ResolvedLocale::En).integer(v)
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
            self.capture_gain_draft(key);
        }
        if fader.committed {
            self.gain_drafts.insert(key, gain);
            self.capture_gain_draft(key);
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
            RichText::new(self.tr(&Message::DiagnosticsConnectionOverride))
                .size(theme::SMALL + 0.5)
                .color(theme::text_2()),
        )
        .id_salt("connection-override")
        .show(ui, |ui| {
            let before = self.hub_address.clone();
            let label = self.tr(&Message::DiagnosticsHttpsAddress);
            widgets::field(
                ui,
                "connection-https-address",
                &label,
                &mut self.hub_address,
                false,
            );
            if before != self.hub_address {
                self.fresh = None;
                self.synced = None;
                self.last_poll = Instant::now() - Duration::from_secs(5);
            }
            widgets::note(ui, self.tr(&Message::DiagnosticsAddressTrustNote));
        });
    }
}
