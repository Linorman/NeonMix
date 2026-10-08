diagnostics-airplay-channel = AirPlay · { $name } · Channel …{ $id }
diagnostics-buffering = Buffering
diagnostics-calculated-from-the-latest-received-packet-this-is = Calculated from the latest received packet. This is not measured end-to-end or speaker latency.
diagnostics-callbacks-over-budget-total = Callbacks over budget (total)
diagnostics-capture-anomaly-count =
    { $count ->
        [one] { $count } anomalie
       *[other] { $count } anomalies
    }
diagnostics-capture-errors-total = Capture errors (total)
diagnostics-capture-frame-value = Capture frames { $value } (total)
diagnostics-capture-frames-total = Capture frames (total)
diagnostics-capture-peak-session-maximum = Capture peak (session maximum)
diagnostics-capture-statistics-appear-after-sending-starts = Capture statistics appear after sending starts
diagnostics-capture-statistics-are-unavailable-start-sending-then-refresh = Capture statistics are unavailable. Start sending, then refresh.
diagnostics-channel-id = Channel …{ $id }
diagnostics-connection-and-processes = Connection and processes
diagnostics-error-count =
    { $count ->
        [one] { $count } error
       *[other] { $count } errors
    }
diagnostics-export-destination-tooltip = Write to { $path }/diagnostics-redacted.json. Keeps only allowed values and status; excludes invitations, tokens, certificates, addresses, device names, and local paths.
diagnostics-export-redacted-diagnostics = Export redacted diagnostics
diagnostics-healthy = Healthy
diagnostics-late-packets-total = Late packets (total)
diagnostics-limited-frames-total = Limited frames (total)
diagnostics-local-background-pid = Local background PID
diagnostics-lost-packets-total = Lost packets (total)
diagnostics-media-network = Media network
diagnostics-media-receiver-statistics-are-unavailable = Media receiver statistics are unavailable.
diagnostics-mixer-queue-estimate-largest-channel = Mixer queue estimate (largest channel)
diagnostics-mixer-queue-estimate-queued-frames-48-khz-it = Mixer queue estimate = queued frames / 48 kHz. It excludes devices, the network, and analog stages; it is not end-to-end audio latency.
diagnostics-no-channels = No channels
diagnostics-no-data-intervals-total = No-data intervals (total)
diagnostics-no-packet-loss = No packet loss
diagnostics-no-receivers = No receivers
diagnostics-not-sending = Not sending
diagnostics-output-errors-total = Output errors (total)
diagnostics-output-fault-count =
    { $count ->
        [one] { $count } output fault detected. Check the physical output device and retry after it recovers.
       *[other] { $count } output faults detected. Check the physical output device and retry after it recovers.
    }
diagnostics-packet-loss-value = Packet loss { $count }
diagnostics-pcm-queue-drops-total = PCM queue drops (total)
diagnostics-pcm-timing-gaps-total = PCM timing gaps (total)
diagnostics-playback-advance = Playback advance
diagnostics-playback-and-output = Playback and output
diagnostics-playback-frame-value = Playback frames { $value } (total)
diagnostics-playback-frames-total = Playback frames (total)
diagnostics-plc-samples-total = PLC samples (total)
diagnostics-receiver-index = Media receiver { $index }
diagnostics-receiver-late-count =
    { $count ->
        [one] { $count } receiver · Late { $late } (total)
       *[other] { $count } receivers · Late { $late } (total)
    }
diagnostics-room-diagnostics-must-be-available-first = Room diagnostics must be available first
diagnostics-room-status = Room status
diagnostics-room-status-is-unavailable-check-sharing-discovery-or = Room status is unavailable. Check sharing, discovery, or your paired identity.
diagnostics-running = Running
diagnostics-send-queue-drops-total = Send queue drops (total)
diagnostics-sender-capture = Sender capture
diagnostics-sent-packets-total = Sent packets (total)
diagnostics-silent-frames-total = Silent frames (total)
diagnostics-source = Source
diagnostics-source-lead-time = Source lead time
diagnostics-state-revision = State revision
diagnostics-status-age = Updated { $seconds } seconds ago
diagnostics-status-age-note = Room status updated { $seconds } seconds ago · Counts are totals for this run
diagnostics-stopped = Stopped
diagnostics-there-are-no-active-channels = There are no active channels.
diagnostics-there-are-no-active-channels-label = There are no active channels
diagnostics-there-are-no-media-receivers = There are no media receivers.
diagnostics-there-are-no-media-receivers-label = There are no media receivers
diagnostics-time-until-playback-after-reception = Time until playback after reception
diagnostics-unavailable = Unavailable
diagnostics-underrun-callback-total = Underrun frames · Callbacks over budget { $count } (total)
diagnostics-underrun-count = Underrun { $count }
diagnostics-underrun-frames-total = Underrun frames (total)
diagnostics-queue-and-drift = { $milliseconds } ms · Drift { $drift } ppm
diagnostics-connection-override = Connection address (troubleshooting)
diagnostics-https-address = HTTPS address (optional)
diagnostics-address-trust-note = Leave blank for automatic discovery. Every address must match the paired certificate and room identity.

diagnostics-observation-age = This source was observed { $seconds } seconds ago.
diagnostics-observation-stale = This source is stale ({ $seconds } seconds); refresh to check.
diagnostics-observation-unavailable-last-good = No new reading for this source. Last good reading: { $seconds } seconds ago.
diagnostics-no-samples = No samples received yet
diagnostics-sampling-silence = Samples received; currently silent
diagnostics-historical-errors = { $count } anomalies recorded in total; current health is evaluated separately.

diagnostics-required-fields-missing = Required readings are missing; refresh diagnostics.

diagnostics-no-recent-samples = No new samples received
