#include <waywallen-bridge/bridge.h>

#include <assert.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

static void test_subscription_codec(void) {
    char* kinds[] = { "audio", "pointer" };
    ww_evt_set_event_subscriptions_t input   = {
        .subscription = {
            .revision = 7,
            .kinds    = { .count = 2, .data = kinds },
        },
    };
    ww_buf_t encoded;
    ww_buf_init(&encoded);
    assert(ww_evt_set_event_subscriptions_encode(&input, &encoded) == 0);

    ww_evt_set_event_subscriptions_t decoded;
    assert(ww_evt_set_event_subscriptions_decode(encoded.data, encoded.len, &decoded) == 0);
    assert(decoded.subscription.revision == 7);
    assert(decoded.subscription.kinds.count == 2);
    assert(strcmp(decoded.subscription.kinds.data[0], "audio") == 0);
    assert(strcmp(decoded.subscription.kinds.data[1], "pointer") == 0);
    ww_evt_set_event_subscriptions_free(&decoded);
    ww_buf_free(&encoded);
}

static void test_request_frame_codec(void) {
    ww_evt_in_request_frame_t input = { 0 };
    ww_buf_t                  encoded;
    ww_buf_init(&encoded);
    assert(ww_evt_in_request_frame_encode(&input, &encoded) == 0);

    ww_evt_in_request_frame_t decoded;
    assert(ww_evt_in_request_frame_decode(encoded.data, encoded.len, &decoded) == 0);
    assert(ww_evt_in_request_frame_expected_fds(&decoded) == 0);
    ww_evt_in_request_frame_free(&decoded);
    const uint8_t trailing = 0;
    assert(ww_evt_in_request_frame_decode(&trailing, 1, &decoded) == WW_ERR_TRAILING);
    ww_buf_free(&encoded);
}

static void test_subscription_ack_view(void) {
    ww_bridge_control_t control                   = { .op = WW_EVT_IN_EVENT_SUBSCRIPTIONS_APPLIED };
    waywallen_event_subscription_result_t* result = &control.u.event_subscriptions_applied.result;
    result->revision                              = 9;
    result->status                                = WAYWALLEN_EVENT_SUBSCRIPTION_STATUS_APPLIED;
    result->kinds.count                           = 1;
    result->kinds.data                            = calloc(1, sizeof(char*));
    result->kinds.data[0]                         = strdup("audio");
    result->reason                                = strdup("");

    assert(result->revision == 9);
    assert(result->kinds.count == 1);
    assert(strcmp(result->kinds.data[0], "audio") == 0);
    ww_bridge_control_free(&control);
}

static void test_audio_helper_validates_complete_windows_and_end(void) {
    ww_bridge_control_t       control = { .op = WW_EVT_IN_AUDIO_WINDOW };
    waywallen_audio_window_t* window  = &control.u.audio_window.window;
    window->subscription_revision     = 4;
    window->generation                = 5;
    window->sequence                  = 6;
    window->captured_at_ns            = 7;
    window->end_sample_frame          = 4096;
    window->format.sample_rate_hz     = WW_BRIDGE_AUDIO_SAMPLE_RATE;
    window->format.channels           = WW_BRIDGE_AUDIO_CHANNELS;
    window->frames                    = WW_BRIDGE_AUDIO_WINDOW_FRAMES;
    window->samples.count             = WW_BRIDGE_AUDIO_SAMPLE_COUNT;
    window->samples.data              = calloc(WW_BRIDGE_AUDIO_SAMPLE_COUNT, sizeof(float));
    for (uint32_t index = 0; index < WW_BRIDGE_AUDIO_SAMPLE_COUNT; ++index) {
        window->samples.data[index] = (float)index / WW_BRIDGE_AUDIO_SAMPLE_COUNT;
    }

    ww_bridge_audio_window_t audio;
    assert(ww_bridge_audio_window_from_control(&control, &audio) == 0);
    assert(audio.subscription_revision == 4);
    assert(audio.frames == WW_BRIDGE_AUDIO_WINDOW_FRAMES);
    assert(audio.samples[1] > 0.0f);

    window->samples.data[0] = NAN;
    assert(ww_bridge_audio_window_from_control(&control, &audio) != 0);
    window->samples.data[0] = 0.0f;
    window->samples.count -= 1;
    assert(ww_bridge_audio_window_from_control(&control, &audio) != 0);
    window->samples.count = 0;
    free(window->samples.data);
    window->samples.data          = NULL;
    window->frames                = 0;
    window->format.sample_rate_hz = 0;
    window->format.channels       = 0;
    window->flags                 = WW_BRIDGE_AUDIO_END_OF_STREAM;
    assert(ww_bridge_audio_window_from_control(&control, &audio) == 0);
    assert(audio.flags == WW_BRIDGE_AUDIO_END_OF_STREAM);
    ww_bridge_control_free(&control);
}

int main(void) {
    test_subscription_codec();
    test_request_frame_codec();
    test_subscription_ack_view();
    test_audio_helper_validates_complete_windows_and_end();
    return 0;
}
