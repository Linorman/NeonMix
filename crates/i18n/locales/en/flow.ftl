flow-native-sender = Native Sender
flow-source-accessible = { $name }, { $kind }, { $detail }, { $gain } dB
flow-hub-accessible = Room “{ $name }”, { $state }, open Mixer
flow-open-mixer = Open Mixer
flow-offline-count = 
    { $count ->
        [one] { $count } other paired device is not sending
       *[other] { $count } other paired devices are not sending
    }
flow-you = You
flow-output-unavailable = Output unavailable
flow-master-muted = Master muted
flow-output-accessible = Physical output “{ $name }”, { $state }
