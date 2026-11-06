// Settings calls that are not a pref set: default zoom, search engine, site
// rules, clearing data (PLAN-settings.md section 5). ABI 5 stubs: each answers
// "nothing happened" so the framework links; WP-B fills them in.

#import <Cocoa/Cocoa.h>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/task/single_thread_task_runner.h"

void baa_profile_set_default_zoom(BaaProfile* profile, double factor) {}

bool baa_profile_set_search_engine(BaaProfile* profile,
                                   const char* name,
                                   const char* keyword,
                                   const char* url) {
  return false;
}

int baa_profile_site_rules(BaaProfile* profile, uint32_t kind, BaaSiteRuleVisitor visit, void* ctx) {
  return -1;
}

bool baa_profile_set_site_rule(BaaProfile* profile, uint32_t kind, const char* pattern, int state) {
  return false;
}

void baa_profile_clear_data(BaaProfile* profile, uint64_t id, int64_t since_ms, uint32_t types) {
  // Answered later, as the contract says.
  base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
      FROM_HERE, base::BindOnce([](uint64_t id) { baa::Callbacks().clear_done(baa::CallbackContext(), id); }, id));
}

void baa_profile_load_unpacked(BaaProfile* profile, uint64_t id, const char* path) {
  base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
      FROM_HERE, base::BindOnce(
                     [](uint64_t id) {
                       baa::Callbacks().load_unpacked_done(baa::CallbackContext(), id, false,
                                                           "Loading unpacked extensions is not implemented");
                     },
                     id));
}
