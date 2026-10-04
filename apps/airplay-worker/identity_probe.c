/* SPDX-License-Identifier: GPL-3.0-or-later; test-only identity loader probe. */
#include <stdio.h>
#include <stdbool.h>
#include "crypto.h"
#include "pairing.h"

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    int generated = -1;
    pairing_t *pairing = pairing_init_generate("001122334455", argv[1], &generated);
    if (!pairing) return 1;
    if (generated != 0) return 3;
    unsigned char public_key[ED25519_KEY_SIZE];
    pairing_get_public_key(pairing, public_key);
    pairing_destroy(pairing);
    ed25519_key_t *key = ed25519_key_generate("001122334455", argv[1], &generated);
    if (!key) return 4;
    unsigned char signature[64];
    ed25519_sign(signature, sizeof(signature), (const unsigned char *) "", 0, key);
    ed25519_key_destroy(key);
    for (size_t i = 0; i < sizeof(public_key); i++) printf("%02x", public_key[i]);
    putchar('\n');
    for (size_t i = 0; i < sizeof(signature); i++) printf("%02x", signature[i]);
    putchar('\n');
    return 0;
}
