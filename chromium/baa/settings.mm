// Settings calls that are not a pref set or a site rule (permissions.mm): default
// zoom, search engine, clearing data (PLAN-settings.md section 5).

#include <cstring>
#include <memory>
#include <string>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/scoped_observation.h"
#include "base/strings/string_util.h"
#include "base/strings/utf_string_conversions.h"
#include "base/time/time.h"
#include "chrome/browser/browsing_data/chrome_browsing_data_remover_constants.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/search_engines/template_url_service_factory.h"
#include "chrome/browser/ui/zoom/chrome_zoom_level_prefs.h"
#include "components/search_engines/template_url.h"
#include "components/search_engines/template_url_data.h"
#include "components/search_engines/template_url_service.h"
#include "components/search_engines/template_url_service_observer.h"
#include "content/public/browser/browsing_data_remover.h"
#include "third_party/blink/public/common/page/page_zoom.h"
#include "url/gurl.h"

namespace baa {
namespace {

// Makes the engine the default; after the list has loaded, if it had not. Owns itself until then.
class EngineApplier : public TemplateURLServiceObserver {
 public:
  // False if Chrome refuses the engine as default now; true when accepted or still waiting.
  static bool Run(TemplateURLService* service, TemplateURLData data) {
    if (service->loaded()) {
      return Apply(service, data);
    }
    new EngineApplier(service, std::move(data));
    return true;
  }

 private:
  EngineApplier(TemplateURLService* service, TemplateURLData data)
      : service_(service), data_(std::move(data)) {
    observation_.Observe(service);
    service->Load();
  }

  void OnTemplateURLServiceChanged() override {
    if (service_->loaded()) {
      Apply(service_, data_);
      delete this;
    }
  }
  // The service goes first at shutdown; nothing to apply to.
  void OnTemplateURLServiceShuttingDown() override { delete this; }

  static bool Apply(TemplateURLService* service, const TemplateURLData& data) {
    std::u16string name = data.short_name(), keyword = data.keyword();
    TemplateURL* engine = nullptr;
    for (TemplateURL* candidate : service->GetTemplateURLs()) {
      if (candidate->url() == data.url()) {
        engine = candidate;
        break;
      }
    }
    // The same site under another URL is the same engine (Chrome's prepopulated Google has
    // many search URLs), so no duplicate.
    if (!engine) {
      engine = service->GetTemplateURLForHost(std::string(GURL(data.url()).host()));
    }
    if (!engine) {
      engine = service->Add(std::make_unique<TemplateURL>(data));
    } else if (engine->prepopulate_id() == 0 && engine->type() == TemplateURL::NORMAL) {
      // Prepopulated engines keep their own names and URLs (Chrome updates them).
      service->ResetTemplateURL(engine, name, keyword, data.url(), engine->suggestions_url());
    }
    // CanMakeDefault says no for the engine that already is the default; set it again anyway so
    // a rename reaches the stored copy.
    if (!engine || (engine != service->GetDefaultSearchProvider() && !service->CanMakeDefault(engine))) {
      return false;
    }
    service->SetUserSelectedDefaultSearchProvider(engine);
    return true;
  }

  raw_ptr<TemplateURLService> service_;
  TemplateURLData data_;
  base::ScopedObservation<TemplateURLService, TemplateURLServiceObserver> observation_{this};
};

// One removal; answers the app and deletes itself.
class Clearing : public content::BrowsingDataRemover::Observer {
 public:
  Clearing(content::BrowsingDataRemover* remover, uint64_t id) : remover_(remover), id_(id) {
    remover->AddObserver(this);  // the remover insists
  }
  void OnBrowsingDataRemoverDone(uint64_t) override {
    remover_->RemoveObserver(this);
    FinishRequest(RequestKind::kClear, id_, true, "");
    delete this;
  }

 private:
  ~Clearing() override = default;
  raw_ptr<content::BrowsingDataRemover> remover_;
  const uint64_t id_;
};

}  // namespace
}  // namespace baa

void baa_profile_set_default_zoom(BaaProfile* profile, double factor) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  if (!chrome_profile || !(factor > 0)) {
    return;
  }
  if (auto* prefs = baa::DefaultsProfile(chrome_profile)->GetZoomLevelPrefs()) {
    prefs->SetDefaultZoomLevelPref(blink::ZoomFactorToZoomLevel(factor));
  }
}

bool baa_profile_set_search_engine(BaaProfile* profile,
                                   const char* name,
                                   const char* keyword,
                                   const char* url) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  if (!chrome_profile || !name || !*name || !url || !strstr(url, "%s")) {
    return false;
  }
  TemplateURLService* service = TemplateURLServiceFactory::GetForProfile(baa::DefaultsProfile(chrome_profile));
  if (!service) {
    return false;
  }
  std::string search_url = url;
  base::ReplaceSubstringsAfterOffset(&search_url, 0, "%s", "{searchTerms}");
  GURL parsed(search_url);
  if (!parsed.is_valid() || !parsed.SchemeIsHTTPOrHTTPS()) {
    return false;
  }
  // "" derives the keyword from the host, as Chrome does for a site search.
  std::u16string word = base::UTF8ToUTF16(keyword ? keyword : "");
  if (word.empty()) {
    word = base::UTF8ToUTF16(parsed.host());
  }
  TemplateURLData data;
  data.SetShortName(base::UTF8ToUTF16(name));
  data.SetKeyword(word);
  data.SetURL(search_url);
  data.favicon_url = TemplateURL::GenerateFaviconURL(parsed);
  data.safe_for_autoreplace = false;
  return baa::EngineApplier::Run(service, std::move(data));
}

void baa_profile_clear_data(BaaProfile* profile, uint64_t id, int64_t since_ms, uint32_t types) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  uint64_t mask = 0;
  if (types & BAA_CLEAR_HISTORY) {
    mask |= chrome_browsing_data_remover::DATA_TYPE_HISTORY;
  }
  if (types & BAA_CLEAR_DOWNLOADS) {
    mask |= content::BrowsingDataRemover::DATA_TYPE_DOWNLOADS;
  }
  if (types & BAA_CLEAR_SITE_DATA) {
    mask |= chrome_browsing_data_remover::DATA_TYPE_SITE_DATA;
  }
  if (types & BAA_CLEAR_CACHE) {
    mask |= content::BrowsingDataRemover::DATA_TYPE_CACHE;
  }
  baa::BeginRequest(baa::RequestKind::kClear, id, chrome_profile);
  if (!chrome_profile || !mask) {
    // Nothing to do or nobody to do it: answered later, as the contract says.
    baa::FinishRequestLater(baa::RequestKind::kClear, id, true, "");
    return;
  }
  base::Time begin = since_ms > 0 ? base::Time::FromMillisecondsSinceUnixEpoch(since_ms) : base::Time();
  // The remover answers from a task, never inside this call.
  content::BrowsingDataRemover* remover = chrome_profile->GetBrowsingDataRemover();
  remover->RemoveAndReply(begin, base::Time::Max(), mask,
                          content::BrowsingDataRemover::ORIGIN_TYPE_UNPROTECTED_WEB,
                          new baa::Clearing(remover, id));
}
