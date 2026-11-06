// Downloads: one observer per profile, one per in-progress item.
// PLAN-chromium-phase4.md step 4.

#import <Cocoa/Cocoa.h>

#include <map>
#include <memory>
#include <string>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/no_destructor.h"
#include "components/download/public/common/download_item.h"
#include "chrome/browser/profiles/profile.h"
#include "content/public/browser/download_item_utils.h"
#include "content/public/browser/download_manager.h"
#include "content/public/browser/web_contents.h"

namespace baa {
namespace {

class ItemWatch;
std::map<uint64_t, ItemWatch*>& Items() {
  static base::NoDestructor<std::map<uint64_t, ItemWatch*>> items;
  return *items;
}

class ItemWatch : public download::DownloadItem::Observer {
 public:
  explicit ItemWatch(download::DownloadItem* item) : item_(item), id_(NextId()) {
    item->AddObserver(this);
    Items()[id_] = this;
  }
  ~ItemWatch() override {
    Items().erase(id_);
    if (item_) {
      item_->RemoveObserver(this);
    }
  }

  void Send() {
    if (!item_) {
      return;
    }
    // Chrome shows no prompt; keeping every file is what CEF did.
    if (item_->IsDangerous()) {
      item_->ValidateDangerousDownload();
    }
    if (item_->IsInsecure()) {
      item_->ValidateInsecureDownload();
    }
    std::string url = item_->GetURL().spec();
    std::string path = (item_->GetTargetFilePath().empty() ? item_->GetFullPath()
                                                           : item_->GetTargetFilePath())
                           .AsUTF8Unsafe();
    BaaDownload download = {};
    download.id = id_;
    download.url = url.c_str();
    download.path = path.c_str();
    download.received_bytes = item_->GetReceivedBytes();
    download.total_bytes = item_->GetTotalBytes();
    download.state = item_->GetState() == download::DownloadItem::IN_PROGRESS ? BAA_DOWNLOAD_IN_PROGRESS
                     : item_->GetState() == download::DownloadItem::COMPLETE   ? BAA_DOWNLOAD_COMPLETE
                                                                                : BAA_DOWNLOAD_CANCELED;
    content::WebContents* wc = content::DownloadItemUtils::GetWebContents(item_);
    // A tab the app knows of, else none (a service worker, a closed tab).
    BaaTab* tab = wc ? TabFor(wc) : nullptr;
    Callbacks().download(CallbackContext(), tab && TabAnnounced(tab) ? IdOf(tab) : 0, &download);
  }

  void Cancel() {
    if (item_ && item_->GetState() == download::DownloadItem::IN_PROGRESS) {
      item_->Cancel(/*user_cancel=*/true);
    }
  }

  void OnDownloadUpdated(download::DownloadItem*) override { Send(); }
  void OnDownloadDestroyed(download::DownloadItem* item) override {
    item->RemoveObserver(this);  // an observer must be out of the list before it dies
    item_ = nullptr;
    delete this;
  }

 private:
  raw_ptr<download::DownloadItem> item_;
  const uint64_t id_;
};

}  // namespace

DownloadWatch::DownloadWatch(Profile* profile) : manager_(profile->GetDownloadManager()) {
  if (manager_) {
    manager_->AddObserver(this);
  }
}

DownloadWatch::~DownloadWatch() {
  if (manager_) {
    manager_->RemoveObserver(this);
  }
}

void DownloadWatch::OnDownloadCreated(content::DownloadManager*, download::DownloadItem* item) {
  // Items loaded from history arrive finished; baa-model keeps its own list.
  if (item->GetState() != download::DownloadItem::IN_PROGRESS) {
    return;
  }
  // Owns itself until the item is destroyed.
  (new ItemWatch(item))->Send();
}

void DownloadWatch::ManagerGoingDown(content::DownloadManager*) {
  manager_ = nullptr;
}

}  // namespace baa

void baa_download_cancel(uint64_t id) {
  auto it = baa::Items().find(id);
  if (it != baa::Items().end()) {
    it->second->Cancel();
  }
}
