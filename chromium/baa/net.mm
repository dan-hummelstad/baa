// baa_profile_fetch: a cookie-less GET through the profile's network stack
// (search suggestions). PLAN-chromium-phase4.md step 6.

#import <Cocoa/Cocoa.h>

#include <map>
#include <memory>
#include <string>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/task/single_thread_task_runner.h"
#include "base/time/time.h"
#include "chrome/browser/profiles/profile.h"
#include "content/public/browser/storage_partition.h"
#include "net/traffic_annotation/network_traffic_annotation.h"
#include "services/network/public/cpp/resource_request.h"
#include "services/network/public/cpp/shared_url_loader_factory.h"
#include "services/network/public/cpp/simple_url_loader.h"
#include "services/network/public/mojom/fetch_api.mojom.h"
#include "url/gurl.h"

namespace {

std::map<uint64_t, std::unique_ptr<network::SimpleURLLoader>>& Loaders() {
  static base::NoDestructor<std::map<uint64_t, std::unique_ptr<network::SimpleURLLoader>>> loaders;
  return *loaders;
}

// A failure that never started a loader: it must not erase one in flight under a reused id.
void Fail(uint64_t id) {
  if (baa::Callbacks().fetch_done) {
    baa::Callbacks().fetch_done(baa::CallbackContext(), id, false, nullptr, 0);
  }
}

void Done(uint64_t id, std::optional<std::string> body) {
  Loaders().erase(id);
  if (!baa::Callbacks().fetch_done) {
    return;
  }
  if (body.has_value()) {
    baa::Callbacks().fetch_done(baa::CallbackContext(), id, true, body->c_str(), body->size());
  } else {
    baa::Callbacks().fetch_done(baa::CallbackContext(), id, false, nullptr, 0);
  }
}

}  // namespace

void baa_profile_fetch(BaaProfile* profile, const char* url, uint64_t id) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  GURL target(url ? url : "");
  // Never answers inside this call, whatever went wrong.
  if (!chrome_profile || !target.is_valid() || !target.SchemeIsHTTPOrHTTPS()) {
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(&Fail, id));
    return;
  }
  net::NetworkTrafficAnnotationTag annotation = net::DefineNetworkTrafficAnnotation("baa_suggest", R"(
      semantics {
        sender: "baa command bar"
        description: "Search suggestions for the text typed in the command bar."
        trigger: "The user types in the command bar."
        data: "The typed text, in the URL of the user's search engine."
        destination: OTHER
      }
      policy {
        cookies_allowed: NO
        setting: "Settings > Search > suggestions."
        policy_exception_justification: "Not implemented."
      })");
  auto request = std::make_unique<network::ResourceRequest>();
  request->url = target;
  request->method = "GET";
  request->credentials_mode = network::mojom::CredentialsMode::kOmit;
  auto loader = network::SimpleURLLoader::Create(std::move(request), annotation);
  loader->SetTimeoutDuration(base::Seconds(10));
  network::SimpleURLLoader* raw = loader.get();
  Loaders()[id] = std::move(loader);
  raw->DownloadToString(chrome_profile->GetDefaultStoragePartition()
                            ->GetURLLoaderFactoryForBrowserProcess()
                            .get(),
                        base::BindOnce(&Done, id), 1024 * 1024);
}
