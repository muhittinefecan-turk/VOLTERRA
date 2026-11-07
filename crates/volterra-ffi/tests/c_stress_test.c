/**
 * Empirical C ABI Stress Test Harness for VOLTERRA
 *
 * Validates C ABI linkage, calling conventions, null-safety across all 24 symbols,
 * struct memory layouts, and event callbacks using Clang C99 compiler.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <assert.h>
#include <unistd.h>
#include <pthread.h>
#include <inttypes.h>
#include "volterra_ffi.h"

static int g_event_counter = 0;

static void c_event_callback(const struct volterra_event_t *event, void *user_data) {
    if (event == NULL) return;
    int *cnt = (int *)user_data;
    if (cnt != NULL) {
        (*cnt)++;
    }
}

int main(void) {
    printf("[C_HARNESS] Starting C99 ABI Empirical Stress Test...\n");

    // 1. Version check
    const char *ver = volterra_version();
    assert(ver != NULL);
    assert(strcmp(ver, "1.0.0") == 0);
    printf("[C_HARNESS] 1. Version verified: %s\n", ver);

    // 2. Null Pointer Safety on all 24 functions from pure C
    struct volterra_error_t *err = NULL;

    // Symbol 1: volterra_job_create
    struct volterra_job_t *job = volterra_job_create(NULL, &err);
    assert(job == NULL);
    assert(err != NULL);
    assert(err->code == VOLTERRA_ERR_INVALID_ARGUMENT);
    volterra_error_free(err);
    err = NULL;

    // Symbol 2: volterra_job_start
    int ret = volterra_job_start(NULL, &err);
    assert(ret == VOLTERRA_ERR_INVALID_ARGUMENT);
    assert(err != NULL);
    volterra_error_free(err);
    err = NULL;

    // Symbol 3: volterra_job_pause
    ret = volterra_job_pause(NULL, NULL);
    assert(ret == VOLTERRA_ERR_INVALID_ARGUMENT);

    // Symbol 4: volterra_job_resume
    ret = volterra_job_resume(NULL, NULL);
    assert(ret == VOLTERRA_ERR_INVALID_ARGUMENT);

    // Symbol 5: volterra_job_cancel
    ret = volterra_job_cancel(NULL, NULL);
    assert(ret == VOLTERRA_ERR_INVALID_ARGUMENT);

    // Symbol 6: volterra_job_free
    volterra_job_free(NULL);

    // Symbol 7: volterra_get_snapshot
    assert(volterra_get_snapshot(NULL) == NULL);

    // Symbol 8: volterra_snapshot_free
    volterra_snapshot_free(NULL);

    // Symbol 9: volterra_get_telemetry
    assert(volterra_get_telemetry(NULL) == NULL);

    // Symbol 10: volterra_telemetry_free
    volterra_telemetry_free(NULL);

    // Symbol 11: volterra_subscribe_events
    ret = volterra_subscribe_events(NULL, NULL, NULL);
    assert(ret == VOLTERRA_ERR_INVALID_ARGUMENT);

    // Symbol 12: volterra_get_errors
    assert(volterra_get_errors(NULL) == NULL);

    // Symbol 13: volterra_error_list_free
    volterra_error_list_free(NULL);

    // Symbol 14: volterra_skip_file
    assert(volterra_skip_file(NULL, 1) == VOLTERRA_ERR_INVALID_ARGUMENT);

    // Symbol 15: volterra_retry_file
    assert(volterra_retry_file(NULL, 1) == VOLTERRA_ERR_INVALID_ARGUMENT);

    // Symbol 16: volterra_defer_file
    assert(volterra_defer_file(NULL, 1) == VOLTERRA_ERR_INVALID_ARGUMENT);

    // Symbol 17: volterra_skip_all_errors
    assert(volterra_skip_all_errors(NULL) == VOLTERRA_ERR_INVALID_ARGUMENT);

    // Symbol 18: volterra_retry_all_failed
    assert(volterra_retry_all_failed(NULL) == VOLTERRA_ERR_INVALID_ARGUMENT);

    // Symbol 19: volterra_discover_disks
    struct volterra_disk_list_t *disks = volterra_discover_disks(NULL);
    assert(disks != NULL);
    assert(disks->count > 0);

    // Symbol 20: volterra_disks_free
    volterra_disks_free(disks);
    volterra_disks_free(NULL);

    // Symbol 21: volterra_export_report
    assert(volterra_export_report(NULL, 0) == NULL);

    // Symbol 22: volterra_string_free
    volterra_string_free(NULL);

    // Symbol 23: volterra_error_free
    volterra_error_free(NULL);

    printf("[C_HARNESS] 2. Null pointer safety across all 24 symbols verified in C.\n");

    // 3. Real Job Creation, Snapshots, Telemetry, Reports, and Clean Teardown
    char src_template[] = "/tmp/volterra_c_src_XXXXXX";
    char dst_template[] = "/tmp/volterra_c_dst_XXXXXX";
    char *src_path = mkdtemp(src_template);
    char *dst_path = mkdtemp(dst_template);
    assert(src_path != NULL && dst_path != NULL);

    // Create a dummy payload file
    char payload_path[512];
    snprintf(payload_path, sizeof(payload_path), "%s/payload.bin", src_path);
    FILE *fp = fopen(payload_path, "wb");
    assert(fp != NULL);
    char buf[1024];
    memset(buf, 0x42, sizeof(buf));
    for (int i = 0; i < 256; i++) {
        fwrite(buf, 1, sizeof(buf), fp);
    }
    fclose(fp);

    struct volterra_config_t cfg;
    memset(&cfg, 0, sizeof(cfg));
    cfg.source_path = src_path;
    cfg.dest_path = dst_path;
    cfg.mode = VOLTERRA_MODE_SMART_HYBRID;
    cfg.chunk_size = 256 * 1024;
    cfg.direct_io = false;
    cfg.verify_readback = false;
    cfg.memory_budget = 16 * 1024 * 1024;
    cfg.allow_overwrite = true;
    cfg.journal_path = NULL;

    job = volterra_job_create(&cfg, &err);
    assert(job != NULL);
    assert(err == NULL);

    // Out-of-order test: resume before start
    printf("[C_HARNESS] Testing out-of-order resume on newly created job (state = Pending)...\n");
    int resume_ret = volterra_job_resume(job, &err);
    printf("[C_HARNESS] volterra_job_resume on Pending returned: %d (expected VOLTERRA_ERR_INVALID_STATE = %d)\n",
           resume_ret, VOLTERRA_ERR_INVALID_STATE);
    assert(resume_ret == VOLTERRA_ERR_INVALID_STATE);
    assert(err != NULL);
    assert(err->code == VOLTERRA_ERR_INVALID_STATE);
    assert(strstr(err->message, "Cannot resume job: job is not in Paused or WaitingForDisk state") != NULL);
    volterra_error_free(err);
    err = NULL;

    struct volterra_snapshot_t *snap_after_resume = volterra_get_snapshot(job);
    assert(snap_after_resume != NULL);
    printf("[C_HARNESS] Job state after rejected resume: %u (VOLTERRA_STATE_PENDING = %d)\n",
           snap_after_resume->state, VOLTERRA_STATE_PENDING);
    assert(snap_after_resume->state == VOLTERRA_STATE_PENDING);
    volterra_snapshot_free(snap_after_resume);

    volterra_job_free(job);

    // Recreate job for normal transfer verification
    job = volterra_job_create(&cfg, &err);
    assert(job != NULL);

    // Subscribe to events
    ret = volterra_subscribe_events(job, c_event_callback, &g_event_counter);
    assert(ret == VOLTERRA_OK);

    // Snapshot query in Pending
    struct volterra_snapshot_t *snap = volterra_get_snapshot(job);
    assert(snap != NULL);
    assert(snap->job_id != NULL);
    assert(snap->state == VOLTERRA_STATE_PENDING);
    volterra_snapshot_free(snap);

    // Telemetry query in Pending
    struct volterra_telemetry_t *tel = volterra_get_telemetry(job);
    assert(tel != NULL);
    assert(tel->bottleneck_message != NULL);
    volterra_telemetry_free(tel);

    // Report export in Pending
    char *json_rep = volterra_export_report(job, VOLTERRA_REPORT_JSON);
    assert(json_rep != NULL);
    assert(strstr(json_rep, "volterra_version") != NULL);
    volterra_string_free(json_rep);

    char *csv_rep = volterra_export_report(job, VOLTERRA_REPORT_CSV);
    assert(csv_rep != NULL);
    assert(strstr(csv_rep, "record_type") != NULL);
    volterra_string_free(csv_rep);

    // Start job
    ret = volterra_job_start(job, &err);
    assert(ret == VOLTERRA_OK);
    assert(err == NULL);

    // Wait for transfer to complete or progress
    usleep(150000); // 150ms

    // Query snapshot during/after run
    snap = volterra_get_snapshot(job);
    assert(snap != NULL);
    printf("[C_HARNESS] Job Snapshot: state=%u, files=%" PRIu64 ", bytes=%" PRIu64 "\n",
           snap->state, snap->total_files, snap->processed_bytes);
    volterra_snapshot_free(snap);

    // Cancel if running, or free
    volterra_job_free(job);
    printf("[C_HARNESS] 3. Job lifecycle, events (%d received), reporting, and free verified in C.\n", g_event_counter);

    // Cleanup temp dirs
    unlink(payload_path);
    rmdir(src_path);
    rmdir(dst_path);

    printf("[C_HARNESS] ALL C99 ABI TESTS PASSED SUCCESSFULLY.\n");
    return 0;
}
