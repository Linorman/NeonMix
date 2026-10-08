events-source-joined = “{ $name }” connected
events-source-status = “{ $name }”: { $status }
events-source-solo = “{ $name }” is soloed
events-source-unsolo = Solo cleared for “{ $name }”
events-source-left = “{ $name }” left the room
events-output-restored = Physical output restored
events-output-lost = Physical output unavailable, waiting for device recovery
events-master-muted = Room master muted
events-master-unmuted = Room master unmuted
events-just-now = Just now
events-seconds-ago =
    { $count ->
        [one] { $count } second ago
       *[other] { $count } seconds ago
    }
events-minutes-ago =
    { $count ->
        [one] { $count } minute ago
       *[other] { $count } minutes ago
    }
events-hours-ago =
    { $count ->
        [one] { $count } hour ago
       *[other] { $count } hours ago
    }
