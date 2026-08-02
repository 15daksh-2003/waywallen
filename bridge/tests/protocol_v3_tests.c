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

static void test_audio_helper_validates_complete_windows_and_end(void) {
    ww_bridge_control_t control                  = { .op = WW_EVT_IN_AUDIO_WINDOW };
    control.u.audio_window.subscription_revision = 4;
    control.u.audio_window.generation            = 5;
    control.u.audio_window.sequence              = 6;
    control.u.audio_window.captured_at_ns        = 7;
    control.u.audio_window.end_sample_frame      = 4096;
    control.u.audio_window.sample_rate_hz        = WW_BRIDGE_AUDIO_SAMPLE_RATE;
    control.u.audio_window.channels              = WW_BRIDGE_AUDIO_CHANNELS;
    control.u.audio_window.frames                = WW_BRIDGE_AUDIO_WINDOW_FRAMES;
    control.u.audio_window.samples.count         = WW_BRIDGE_AUDIO_SAMPLE_COUNT;
    control.u.audio_window.samples.data = calloc(WW_BRIDGE_AUDIO_SAMPLE_COUNT, sizeof(float));
    for (uint32_t index = 0; index < WW_BRIDGE_AUDIO_SAMPLE_COUNT; ++index) {
        control.u.audio_window.samples.data[index] = (float)index / WW_BRIDGE_AUDIO_SAMPLE_COUNT;
    }

    ww_bridge_audio_window_t audio;
    assert(ww_bridge_audio_window_from_control(&control, &audio) == 0);
    assert(audio.subscription_revision == 4);
    assert(audio.frames == WW_BRIDGE_AUDIO_WINDOW_FRAMES);
    assert(audio.samples[1] > 0.0f);

    control.u.audio_window.samples.data[0] = NAN;
    assert(ww_bridge_audio_window_from_control(&control, &audio) != 0);
    control.u.audio_window.samples.data[0] = 0.0f;
    control.u.audio_window.samples.count -= 1;
    assert(ww_bridge_audio_window_from_control(&control, &audio) != 0);
    control.u.audio_window.samples.count = 0;
    free(control.u.audio_window.samples.data);
    control.u.audio_window.samples.data   = NULL;
    control.u.audio_window.frames         = 0;
    control.u.audio_window.sample_rate_hz = 0;
    control.u.audio_window.channels       = 0;
    control.u.audio_window.flags          = WW_BRIDGE_AUDIO_END_OF_STREAM;
    assert(ww_bridge_audio_window_from_control(&control, &audio) == 0);
    assert(audio.flags == WW_BRIDGE_AUDIO_END_OF_STREAM);
    ww_bridge_control_free(&control);
}

int main(void) {
    test_subscription_codec();
    test_subscription_ack_transfer();
    test_audio_helper_validates_complete_windows_and_end();
    return 0;
}
