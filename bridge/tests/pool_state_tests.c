#include "pool_internal.h"

#include <assert.h>
#include <errno.h>
#include <string.h>

static void init_pool(ww_pool_t* pool) {
    memset(pool, 0, sizeof(*pool));
    assert(pthread_mutex_init(&pool->release_mutex, NULL) == 0);
    assert(pthread_cond_init(&pool->release_cond, NULL) == 0);
    pool->has_directive      = true;
    pool->n_slots            = 1;
    pool->bind_generation    = 7;
    pool->release_slot_count = 1;
}

static void destroy_pool_state(ww_pool_t* pool) {
    assert(pthread_cond_destroy(&pool->release_cond) == 0);
    assert(pthread_mutex_destroy(&pool->release_mutex) == 0);
}

static void test_pending_release_notification(void) {
    ww_pool_t pool;
    init_pool(&pool);
    pool.pending_submit = (ww_pool_pending_submit_t) {
        .bind_generation = 7,
        .release_point   = 12,
        .slot_index      = 0,
        .active          = true,
    };

    assert(ww_bridge_pool_report_release(&pool, 7, 0, 12, WW_POOL_RELEASE_NO_CONSUMERS) == 0);
    assert(pool.pending_submit.outcome_known);
    assert(pool.pending_submit.outcome == WW_POOL_RELEASE_NO_CONSUMERS);
    assert(ww_bridge_pool_report_release(&pool, 7, 0, 12, WW_POOL_RELEASE_NO_CONSUMERS) == 0);
    assert(ww_bridge_pool_report_release(&pool, 7, 0, 12, WW_POOL_RELEASE_FORCED) == -EPROTO);

    destroy_pool_state(&pool);
}

static void test_committed_release_notification(void) {
    ww_pool_t pool;
    init_pool(&pool);
    pool.last_release_point[0] = 14;

    assert(ww_bridge_pool_report_release(&pool, 7, 0, 14, WW_POOL_RELEASE_CONSUMER_RELEASED) == 0);
    assert(pool.release_proofs[0].known);
    assert(pool.release_proofs[0].bind_generation == 7);
    assert(pool.release_proofs[0].release_point == 14);
    assert(pool.release_proofs[0].outcome == WW_POOL_RELEASE_CONSUMER_RELEASED);
    assert(ww_bridge_pool_report_release(&pool, 6, 0, 14, WW_POOL_RELEASE_CONSUMER_RELEASED) ==
           -ESTALE);

    destroy_pool_state(&pool);
}

static void test_send_failure_does_not_commit_release_point(void) {
    ww_pool_t pool;
    init_pool(&pool);
    pool.release_point         = 20;
    pool.last_release_point[0] = 19;

    ww_pool_slot_submit_result_t result;
    assert(ww_bridge_pool_submit_slot_for_render(&pool, -1, 0, -1, &result) == 0);
    assert(result.status == WW_POOL_SLOT_SUBMIT_SESSION_LOST);
    assert(result.release_point == 21);
    assert(result.error_code != 0);
    assert(pool.session_lost);
    assert(pool.release_point == 20);
    assert(pool.last_release_point[0] == 19);
    assert(! pool.pending_submit.active);

    destroy_pool_state(&pool);
}

int main(void) {
    test_pending_release_notification();
    test_committed_release_notification();
    test_send_failure_does_not_commit_release_point();
    return 0;
}
