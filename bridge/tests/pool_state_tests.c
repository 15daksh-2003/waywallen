#include "pool_internal.h"

#include <assert.h>
#include <errno.h>
#include <string.h>

static int populate_slot_view(ww_pool_t* pool, uint32_t slot_index, ww_pool_slot_t* out_slot) {
    (void)pool;
    out_slot->gl_export_fbo = slot_index + 100;
    return 0;
}

static const struct ww_pool_backend_ops TEST_OPS = {
    .populate_slot_view = populate_slot_view,
};

static void init_pool(ww_pool_t* pool, uint32_t slot_count) {
    memset(pool, 0, sizeof(*pool));
    assert(pthread_mutex_init(&pool->release_mutex, NULL) == 0);
    pool->has_directive       = true;
    pool->n_slots             = slot_count;
    pool->bind_generation     = 7;
    pool->release_slot_count  = slot_count;
    pool->next_acquire_serial = 1;
    pool->ops                 = &TEST_OPS;
}

static void destroy_pool_state(ww_pool_t* pool) {
    assert(pthread_mutex_destroy(&pool->release_mutex) == 0);
}

static ww_pool_slot_identity_t identity(uint64_t previous_release_point, uint64_t serial) {
    return (ww_pool_slot_identity_t) {
        .bind_generation        = 7,
        .slot_index             = 0,
        .previous_release_point = previous_release_point,
        .acquire_serial         = serial,
    };
}

static void test_abort_requires_exact_identity(void) {
    ww_pool_t pool;
    init_pool(&pool, 1);
    ww_pool_slot_identity_t acquired = identity(12, 3);
    pool.pending_acquire             = (ww_pool_pending_acquire_t) {
        .identity = acquired,
        .active   = true,
    };

    ww_pool_slot_identity_t stale = identity(12, 4);
    assert(ww_bridge_pool_abort_acquired_slot(&pool, &stale) == -ESTALE);
    assert(pool.pending_acquire.active);
    assert(ww_bridge_pool_abort_acquired_slot(&pool, &acquired) == 0);
    assert(! pool.pending_acquire.active);

    destroy_pool_state(&pool);
}

static void test_send_failure_does_not_commit_release_point(void) {
    ww_pool_t pool;
    init_pool(&pool, 1);
    pool.release_point               = 20;
    pool.last_release_point[0]       = 19;
    ww_pool_slot_identity_t acquired = identity(19, 5);
    pool.pending_acquire             = (ww_pool_pending_acquire_t) {
        .identity = acquired,
        .active   = true,
    };

    ww_pool_slot_submit_result_t result;
    assert(ww_bridge_pool_submit_acquired_slot(&pool, -1, &acquired, -1, &result) == 0);
    assert(result.status == WW_POOL_SLOT_SUBMIT_SESSION_LOST);
    assert(result.release_point == 21);
    assert(result.error_code != 0);
    assert(pool.session_lost);
    assert(pool.release_point == 20);
    assert(pool.last_release_point[0] == 19);
    assert(! pool.pending_submit.active);
    assert(! pool.pending_acquire.active);

    destroy_pool_state(&pool);
}

static void test_acquire_any_scans_fresh_slots(void) {
    const uint32_t slot_counts[] = { 1, 2, 3, WW_POOL_MAX_SLOT_COUNT };
    for (size_t count_index = 0; count_index < sizeof(slot_counts) / sizeof(slot_counts[0]);
         ++count_index) {
        ww_pool_t pool;
        init_pool(&pool, slot_counts[count_index]);
        for (uint32_t expected = 0; expected < slot_counts[count_index]; ++expected) {
            ww_pool_slot_acquire_result_t result;
            assert(ww_bridge_pool_try_acquire_any_for_render(&pool, &result) == 0);
            assert(result.status == WW_POOL_SLOT_ACQUIRE_READY_UNUSED);
            assert(result.identity.slot_index == expected);
            assert(result.slot.index == expected);
            assert(result.slot.gl_export_fbo == expected + 100);
            assert(ww_bridge_pool_abort_acquired_slot(&pool, &result.identity) == 0);
        }
        destroy_pool_state(&pool);
    }
}

static void test_acquire_any_skips_submitted_slot_for_fresh_slot(void) {
    ww_pool_t pool;
    init_pool(&pool, 3);
    pool.last_release_point[0] = 9;

    ww_pool_slot_acquire_result_t result;
    assert(ww_bridge_pool_try_acquire_any_for_render(&pool, &result) == 0);
    assert(result.status == WW_POOL_SLOT_ACQUIRE_READY_UNUSED);
    assert(result.identity.slot_index == 1);
    assert(ww_bridge_pool_abort_acquired_slot(&pool, &result.identity) == 0);

    destroy_pool_state(&pool);
}

int main(void) {
    test_abort_requires_exact_identity();
    test_send_failure_does_not_commit_release_point();
    test_acquire_any_scans_fresh_slots();
    test_acquire_any_skips_submitted_slot_for_fresh_slot();
    return 0;
}
