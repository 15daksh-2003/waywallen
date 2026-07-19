#include <waywallen-bridge/bridge.h>

#include <assert.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

static void test_subscription_codec(void) {
    char*                            kinds[] = { "audio", "pointer" };
    ww_evt_set_event_subscriptions_t input   = {
        .revision = 7,
        .kinds    = { .count = 2, .data = kinds },
    };
    ww_buf_t encoded;
    ww_buf_init(&encoded);
    assert(ww_evt_set_event_subscriptions_encode(&input, &encoded) == 0);

    ww_evt_set_event_subscriptions_t decoded;
    assert(ww_evt_set_event_subscriptions_decode(encoded.data, encoded.len, &decoded) == 0);
    assert(decoded.revision == 7);
    assert(decoded.kinds.count == 2);
    assert(strcmp(decoded.kinds.data[0], "audio") == 0);
    assert(strcmp(decoded.kinds.data[1], "pointer") == 0);
    ww_evt_set_event_subscriptions_free(&decoded);
    ww_buf_free(&encoded);
}

static void test_subscription_ack_transfer(void) {
    ww_bridge_control_t control = { .op = WW_EVT_IN_EVENT_SUBSCRIPTIONS_APPLIED };
    control.u.event_subscriptions_applied.revision      = 9;
    control.u.event_subscriptions_applied.status        = WW_BRIDGE_SUBSCRIPTION_APPLIED;
    control.u.event_subscriptions_applied.kinds.count   = 1;
    control.u.event_subscriptions_applied.kinds.data    = calloc(1, sizeof(char*));
    control.u.event_subscriptions_applied.kinds.data[0] = strdup("audio");
    control.u.event_subscriptions_applied.reason        = strdup("");

    ww_bridge_event_subscriptions_applied_t applied;
    assert(ww_bridge_event_subscriptions_applied_from_control(&control, &applied) == 0);
    assert(applied.revision == 9);
    assert(applied.kinds.count == 1);
    assert(strcmp(applied.kinds.data[0], "audio") == 0);
    ww_bridge_control_free(&control);
    ww_bridge_event_subscriptions_applied_free(&applied);
}

static void test_audio_helper_validates_fixed_normalized_arrays(void) {
    ww_bridge_control_t control                    = { .op = WW_EVT_IN_AUDIO_SPECTRUM };
    control.u.audio_spectrum.subscription_revision = 4;
    control.u.audio_spectrum.generation            = 5;
    control.u.audio_spectrum.sequence              = 6;
    control.u.audio_spectrum.captured_at_ns        = 7;
    control.u.audio_spectrum.left.count            = WW_BRIDGE_AUDIO_SPECTRUM_BINS;
    control.u.audio_spectrum.right.count           = WW_BRIDGE_AUDIO_SPECTRUM_BINS;
    control.u.audio_spectrum.left.data  = calloc(WW_BRIDGE_AUDIO_SPECTRUM_BINS, sizeof(float));
    control.u.audio_spectrum.right.data = calloc(WW_BRIDGE_AUDIO_SPECTRUM_BINS, sizeof(float));
    for (uint32_t index = 0; index < WW_BRIDGE_AUDIO_SPECTRUM_BINS; ++index) {
        control.u.audio_spectrum.left.data[index]  = (float)index / 63.0f;
        control.u.audio_spectrum.right.data[index] = 1.0f - (float)index / 63.0f;
    }

    ww_bridge_audio_spectrum_t audio;
    assert(ww_bridge_audio_spectrum_from_control(&control, &audio) == 0);
    assert(audio.subscription_revision == 4);
    assert(audio.left[63] == 1.0f);
    assert(audio.right[63] == 0.0f);

    control.u.audio_spectrum.left.data[0] = NAN;
    assert(ww_bridge_audio_spectrum_from_control(&control, &audio) != 0);
    control.u.audio_spectrum.left.data[0] = 0.0f;
    control.u.audio_spectrum.right.count -= 1;
    assert(ww_bridge_audio_spectrum_from_control(&control, &audio) != 0);
    ww_bridge_control_free(&control);
}

int main(void) {
    test_subscription_codec();
    test_subscription_ack_transfer();
    test_audio_helper_validates_fixed_normalized_arrays();
    return 0;
}
