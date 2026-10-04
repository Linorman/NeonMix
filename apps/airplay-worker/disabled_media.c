/* GPL-3.0-or-later; inert link boundary. No video socket, decoder, or fetch exists. */
#include "raop_rtp_mirror.h"
#include "airplay_video.h"
raop_rtp_mirror_t *raop_rtp_mirror_init(logger_t *l, raop_callbacks_t *c, raop_ntp_t *n, const char *r, int len, const unsigned char *key) { return 0; }
void raop_rtp_mirror_init_aes(raop_rtp_mirror_t *r, uint64_t *id) {}
void raop_rtp_mirror_start(raop_rtp_mirror_t *r, unsigned short *p, uint8_t f) {}
void raop_rtp_mirror_stop(raop_rtp_mirror_t *r) {}
void raop_rtp_mirror_destroy(raop_rtp_mirror_t *r) {}
void airplay_video_destroy(airplay_video_t *v) {}
