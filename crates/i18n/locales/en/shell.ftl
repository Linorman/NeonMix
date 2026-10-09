shell-page-live = Live

shell-page-mixer = Mixer

shell-page-hub = Hub settings

shell-page-sender = Sender

shell-page-devices = Devices

shell-page-diagnostics = Diagnostics

shell-page-about = Settings

shell-processing = Working…

shell-stop-after-start = Sending will stop as soon as the start operation completes…

shell-quitting = Quitting background audio…

shell-stopping = Stopping sending…

shell-stopped = Sending stopped

shell-connected = Background connected

shell-recovered = Room state refreshed

shell-completed = Operation completed

shell-partial-configuration = Some receiver identities were saved, but configuration is incomplete. Check the receivers and retry.

shell-session-ended = Device settings saved. The original session ended; the new connection was left unchanged.

shell-durability = Configuration written, but disk sync was not confirmed. Check storage.

shell-entry-pending = Settings saved, but the receivers have not finished updating. Check Diagnostics and receiver status.

shell-media-pending = Settings saved. Syncing to the audio engine.

shell-exported = Redacted diagnostics exported to diagnostics-redacted.json

shell-forgot-pairing = Local pairing deleted and output disabled. Pair again, then enable or add the output manually.

shell-connecting = Connecting to background…

shell-preview = Visual preview mode

shell-preview-read-failed = Could not read the preview recording.

shell-tray-unavailable = Tray unavailable. Closing the window minimizes it; background audio continues.

shell-quit = Quit background audio
    .short = Quit audio

shell-quit-consequence = Stop sharing, sending, and all audio connections for this instance. Closing the window keeps audio running; quitting background audio stops it.

shell-revoke = Revoke pairing

shell-forget = Delete local pairing

shell-remove-output = Delete output binding

shell-restored = Restored: { $change }

shell-default-room = Living room

shell-default-sender = My computer

shell-hide = Hide window
    .short = Hide

shell-stop-sending = Stop sending

shell-role-admin = Administrator

shell-role-controller = Room controller

shell-role-member = Member

shell-unverified = Identity unverified

shell-unpaired = Not paired

shell-identity = Control identity

shell-current-identity = Current identity: { $role }

shell-no-room = No room connected

shell-sharing = Sharing

shell-not-sharing = Not sharing

shell-stale = State out of date

shell-room-connected = Connected

shell-room-disconnected = Disconnected

shell-room-summary =
    { $count ->
        [one] { $room } · { $state } · { $count } input
       *[other] { $room } · { $state } · { $count } inputs
    }

shell-room-tooltip = Room and status for the current control identity

shell-font-missing = CJK font unavailable. Set NEONMIX_CJK_FONT and reopen to display Chinese names.

shell-quick-actions = Quick actions  { $shortcut }

shell-quick-tooltip = Find actions, navigate pages, and control channels

shell-master-unmute = Unmute master

shell-master-mute = Mute master

shell-output-tooltip = Room output level after limiting. Click to open Mixer.

shell-master-gain = Master volume { $previous } → { $next } dB

shell-sending = Sending

shell-hub-sharing = Hub sharing

shell-hub-stopped = Hub not sharing

shell-hub-unknown = Hub unavailable

shell-preview-badge = Preview

shell-online = Background online

shell-offline = Background offline

shell-restore-button = Undo  { $shortcut }

shell-restore-tooltip = Restore values from before the last mix change

shell-adjusted = Changed: { $change }

shell-local-admin = Local administrator

shell-paired-device = Paired device

shell-copy-error-code = Copy error code: { $code }

shell-stop-unconfirmed = Stop result unconfirmed. Refresh status or retry Stop.

shell-sharing-stopped = Sharing stopped

shell-media-applied = Settings saved and applied to the audio engine.
shell-media-stalled = Settings saved. Audio is still waiting; check the output or restart sharing.
shell-media-unknown = Settings saved. Audio application is not yet confirmed.
shell-media-superseded = Settings saved, but the value has since changed.
shell-group-monitor = Monitor
shell-group-connect = Connect
shell-group-system = System
shell-search-placeholder = Search or run an action
shell-sending-now = This device is sending
shell-account-menu = Identity and background
shell-switch-identity = Switch identity
shell-open-room = Open room settings
shell-room-state =
    { $count ->
        [one] { $state } · 1 input
       *[other] { $state } · { $count } inputs
    }
