// Settings calls that are not a pref set or a site rule (permissions.mm): default
// zoom, search engine, clearing data, loading unpacked extensions
// (PLAN-settings.md section 5).

#import <Cocoa/Cocoa.h>

#include <cstring>
#include <map>
#include <memory>
#include <string>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/files/file_path.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/strings/string_util.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "base/time/time.h"
#include "chrome/browser/browsing_data/chrome_browsing_data_remover_constants.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/search_engines/template_url_service_factory.h"
#include "chrome/browser/ui/zoom/chrome_zoom_level_prefs.h"
#include "components/search_engines/template_url.h"
#include "components/search_engines/template_url_data.h"
#include "components/search_engines/template_url_service.h"
#include "content/public/browser/browsing_data_remover.h"
#include "extensions/browser/unpacked_installer.h"
#include "extensions/common/extension.h"
#include "third_party/blink/public/common/page/page_zoom.h"
#include "url/gurl.h"

namespace baa {
namespace {

// Applies the engine once the list has loaded; owns itself until then.
class EngineApplier {
 public:
  static void Run(TemplateURLService* service, TemplateURLData data) {
    if (service->loaded()) {
      Apply(service, data);
      return;
    }
    auto* self = new EngineApplier(service, std::move(data));
    self->subscription_ = service->RegisterOnLoadedCallback(
        base::BindOnce(&EngineApplier::Loaded, base::Unretained(self)));
    service->Load();
  }

 private:
  EngineApplier(TemplateURLService* service, TemplateURLData data)
      : service_(service), data_(std::move(data)) {}

  void Loaded() {
    Apply(service_, data_);
    // Deleting inside the callback list's own run is not safe.
    base::SingleThreadTaskRunner::GetCurrentDefault()->DeleteSoon(FROM_HERE, this);
  }

  static void Apply(TemplateURLService* service, const TemplateURLData& data) {
    TemplateURL* engine = nullptr;
    for (TemplateURL* candidate : service->GetTemplateURLs()) {
      if (candidate->url() == data.url()) {
        engine = candidate;
        break;
      }
    }
    if (!engine) {
      engine = service->Add(std::make_unique<TemplateURL>(data));
    }
    if (engine && service->CanMakeDefault(engine)) {
      service->SetUserSelectedDefaultSearchProvider(engine);
    }
  }

  raw_ptr<TemplateURLService> service_;
  TemplateURLData data_;
  base::CallbackListSubscription subscription_;
};

// Calls the app has not been answered yet, by the caller's id, and the profile each runs
// in: a profile that closes mid-operation answers them itself (AnswerPendingOf), and the
// late real completion then finds nothing.
std::map<uint64_t, Profile*>& PendingClears() {
  static base::NoDestructor<std::map<uint64_t, Profile*>> pending;
  return *pending;
}
std::map<uint64_t, Profile*>& PendingLoads() {
  static base::NoDestructor<std::map<uint64_t, Profile*>> pending;
  return *pending;
}
bool Take(std::map<uint64_t, Profile*>& pending, uint64_t id) {
  return pending.erase(id) > 0;
}

void AnswerClear(uint64_t id);
void AnswerLoad(uint64_t id, bool ok, std::string message);

// One removal; answers the app from a task and deletes itself.
class Clearing : public content::BrowsingDataRemover::Observer {
 public:
  Clearing(content::BrowsingDataRemover* remover, uint64_t id) : remover_(remover), id_(id) {
    remover->AddObserver(this);  // the remover insists
  }
  void OnBrowsingDataRemoverDone(uint64_t) override {
    remover_->RemoveObserver(this);
    if (Take(PendingClears(), id_)) {
      Callbacks().clear_done(CallbackContext(), id_);
    }
    delete this;
  }

 private:
  ~Clearing() override = default;
  raw_ptr<content::BrowsingDataRemover> remover_;
  const uint64_t id_;
};

void AnswerClear(uint64_t id) {
  Callbacks().clear_done(CallbackContext(), id);
}

void AnswerLoad(uint64_t id, bool ok, std::string message) {
  Callbacks().load_unpacked_done(CallbackContext(), id, ok, message.c_str());
}

void AnswerFromPending(uint64_t id, bool ok, std::string message) {
  if (Take(PendingLoads(), id)) {
    AnswerLoad(id, ok, std::move(message));
  }
}

void AnswerPendingOfImpl(Profile* profile) {
  auto runner = base::SingleThreadTaskRunner::GetCurrentDefault();
  for (auto it = PendingClears().begin(); it != PendingClears().end();) {
    if (it->second == profile) {
      runner->PostTask(FROM_HERE, base::BindOnce(&AnswerClear, it->first));
      it = PendingClears().erase(it);
    } else {
      ++it;
    }
  }
  for (auto it = PendingLoads().begin(); it != PendingLoads().end();) {
    if (it->second == profile) {
      runner->PostTask(FROM_HERE,
                       base::BindOnce(&AnswerLoad, it->first, false, std::string("The profile closed")));
      it = PendingLoads().erase(it);
    } else {
      ++it;
    }
  }
}

// OTR profiles ignore default content settings and share the parent's search engines, so the
// profile-wide defaults go to the parent.
Profile* DefaultsOf(Profile* profile) {
  return profile->IsOffTheRecord() ? profile->GetOriginalProfile() : profile;
}

}  // namespace

void AnswerPendingOf(Profile* profile) {
  AnswerPendingOfImpl(profile);
}
}  // namespace baa

void baa_profile_set_default_zoom(BaaProfile* profile, double factor) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  if (!chrome_profile || !(factor > 0)) {
    return;
  }
  if (auto* prefs = baa::DefaultsOf(chrome_profile)->GetZoomLevelPrefs()) {
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
  TemplateURLService* service = TemplateURLServiceFactory::GetForProfile(baa::DefaultsOf(chrome_profile));
  if (!service) {
    return false;
  }
  std::string search_url = url;
  base::ReplaceSubstringsAfterOffset(&search_url, 0, "%s", "{searchTerms}");
  GURL parsed(search_url);
  if (!parsed.is_valid() || !parsed.SchemeIsHTTPOrHTTPS()) {
    return false;
  }
  std::u16string short_name = base::UTF8ToUTF16(name);
  // "" derives the keyword from the host, as Chrome does for a site search.
  std::u16string word = base::UTF8ToUTF16(keyword ? keyword : "");
  if (word.empty()) {
    word = base::UTF8ToUTF16(parsed.host());
  }
  TemplateURLData data;
  data.SetShortName(short_name);
  data.SetKeyword(word);
  data.SetURL(search_url);
  data.favicon_url = TemplateURL::GenerateFaviconURL(parsed);
  data.safe_for_autoreplace = false;
  baa::EngineApplier::Run(service, std::move(data));
  return true;
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
  if (!chrome_profile || !mask) {
    // Nothing to do or nobody to do it: answered later, as the contract says.
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(&baa::AnswerClear, id));
    return;
  }
  base::Time begin = since_ms > 0 ? base::Time::FromMillisecondsSinceUnixEpoch(since_ms) : base::Time();
  // The remover answers from a task, never inside this call.
  baa::PendingClears()[id] = chrome_profile;
  content::BrowsingDataRemover* remover = chrome_profile->GetBrowsingDataRemover();
  remover->RemoveAndReply(begin, base::Time::Max(), mask,
                          content::BrowsingDataRemover::ORIGIN_TYPE_UNPROTECTED_WEB,
                          new baa::Clearing(remover, id));
}

void baa_profile_load_unpacked(BaaProfile* profile, uint64_t id, const char* path) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  if (!chrome_profile || !path || !*path) {
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(&baa::AnswerLoad, id, false, std::string("No such profile or path")));
    return;
  }
  auto installer = extensions::UnpackedInstaller::Create(chrome_profile);
  baa::PendingLoads()[id] = chrome_profile;
  installer->set_be_noisy_on_failure(false);
  // Posted: the installer reports some failures before Load returns.
  installer->set_completion_callback(base::BindOnce(
      [](uint64_t id, const extensions::Extension* extension, const base::FilePath&, const std::u16string& error) {
        std::string message = extension ? extension->id() : base::UTF16ToUTF8(error);
        if (message.empty()) {
          message = "Could not load the extension";
        }
        base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
            FROM_HERE, base::BindOnce(&baa::AnswerFromPending, id, extension != nullptr, std::move(message)));
      },
      id));
  installer->Load(base::FilePath(path));
}
