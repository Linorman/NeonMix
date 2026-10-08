// SPDX-License-Identifier: GPL-3.0-or-later
// Windows parent/child NamedPipe regression using the production writer.
#include "platform.h"
#include <iostream>

int main(int argc, char **argv) {
#ifdef _WIN32
    try {
        if (argc != 3) return 2;
        const std::string mode=argv[2];
        if (mode!="throughput" && mode!="stalled" && mode!="stop" && mode!="peer_closed") return 2;
        platform::Runtime runtime;
        platform::MediaPipe pipe;
        pipe.connect(argv[1]);
        std::atomic<bool> running{true};
        std::thread stopper;
        if (mode=="stop") stopper=std::thread([&]{
            std::this_thread::sleep_for(std::chrono::milliseconds(50));
            running=false;
        });
        const auto start=std::chrono::steady_clock::now();
        std::vector<uint8_t> packet(3176,0x5a);
        unsigned complete=0;
        // A pending write can complete successfully as the peer closes its
        // handle. The next write must observe closure without hanging.
        const unsigned count=mode=="throughput"?600:mode=="peer_closed"?3:2;
        for (; complete<count; ++complete) {
            if (pipe.write(packet.data(),packet.size(),running)!=(platform::Count)packet.size()) break;
        }
        const auto elapsed=std::chrono::duration_cast<std::chrono::microseconds>(
            std::chrono::steady_clock::now()-start).count();
        if (stopper.joinable()) stopper.join();
        std::cout<<"{\"packets\":"<<complete<<",\"elapsed_us\":"<<elapsed<<"}\n";
        return 0;
    } catch(const std::exception &e) {
        std::cerr<<e.what()<<'\n';
        return 1;
    }
#else
    return 2;
#endif
}
