// A tab's media session reported as `media`, and baa_tab_media_action.
// PLAN-m4-features.md §0 and C. The same session Chrome's own media controls
// (global media controls, the hardware media keys) drive.

#include <map>
#include <memory>
#include <optional>
#include <string>
#include <tuple>
#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/containers/flat_map.h"
#include "base/no_destructor.h"
#include "base/strings/utf_string_conversions.h"
#include "content/public/browser/media_session.h"
#include "content/public/browser/web_contents.h"
#include "mojo/public/cpp/bindings/receiver.h"
#include "services/media_session/public/cpp/media_image.h"
#include "services/media_session/public/cpp/media_metadata.h"
#include "services/media_session/public/cpp/media_position.h"
#include "services/media_session/public/mojom/media_session.mojom.h"

namespace baa {
namespace {

using media_session::mojom::MediaSessionAction;

class MediaWatch : public media_session::mojom::MediaSessionObserver {
 public:
  MediaWatch(uint64_t tab, content::WebContents* wc) : tab_(tab) {
    content::MediaSession::Get(wc)->AddObserver(receiver_.BindNewPipeAndPassRemote());
  }

  // media_session::mojom::MediaSessionObserver:
  void MediaSessionInfoChanged(media_session::mojom::MediaSessionInfoPtr info) override {
    playing_ = info && info->playback_state == media_session::mojom::MediaPlaybackState::kPlaying;
    Send();
  }
  void MediaSessionMetadataChanged(
      const std::optional<media_session::MediaMetadata>& metadata) override {
    title_ = metadata ? base::UTF16ToUTF8(metadata->title) : "";
    artist_ = metadata ? base::UTF16ToUTF8(metadata->artist) : "";
    Send();
  }
  void MediaSessionActionsChanged(const std::vector<MediaSessionAction>& actions) override {
    actions_ = 0;
    for (MediaSessionAction action : actions) {
      switch (action) {
        case MediaSessionAction::kPlay:
          actions_ |= BAA_MEDIA_PLAY;
          break;
        case MediaSessionAction::kPause:
          actions_ |= BAA_MEDIA_PAUSE;
          break;
        case MediaSessionAction::kNextTrack:
          actions_ |= BAA_MEDIA_NEXT;
          break;
        case MediaSessionAction::kPreviousTrack:
          actions_ |= BAA_MEDIA_PREVIOUS;
          break;
        case MediaSessionAction::kEnterPictureInPicture:
          actions_ |= BAA_MEDIA_PICTURE_IN_PICTURE;
          break;
        default:
          break;
      }
    }
    Send();
  }
  void MediaSessionImagesChanged(
      const base::flat_map<media_session::mojom::MediaSessionImageType,
                           std::vector<media_session::MediaImage>>& images) override {
    auto it = images.find(media_session::mojom::MediaSessionImageType::kArtwork);
    artwork_ = it != images.end() && !it->second.empty() ? it->second.front().src.spec() : "";
    Send();
  }
  void MediaSessionPositionChanged(
      const std::optional<media_session::MediaPosition>&) override {}

 private:
  // Only what changed: AddObserver replays every field at once.
  void Send() {
    auto now = std::tie(title_, artist_, artwork_, playing_, actions_);
    if (now == sent_ || !TabAnnounced(TabFromId(tab_))) {
      return;
    }
    sent_ = now;
    BaaMedia media = {title_.c_str(), artist_.c_str(), artwork_.c_str(), playing_, actions_};
    Callbacks().media(CallbackContext(), tab_, &media);
  }

  const uint64_t tab_;
  std::string title_, artist_, artwork_;
  bool playing_ = false;
  uint32_t actions_ = 0;
  std::tuple<std::string, std::string, std::string, bool, uint32_t> sent_;
  mojo::Receiver<media_session::mojom::MediaSessionObserver> receiver_{this};
};

std::map<uint64_t, std::unique_ptr<MediaWatch>>& Watches() {
  static base::NoDestructor<std::map<uint64_t, std::unique_ptr<MediaWatch>>> watches;
  return *watches;
}

}  // namespace

void WatchMedia(uint64_t tab, content::WebContents* wc) {
  Watches()[tab] = std::make_unique<MediaWatch>(tab, wc);
}

void UnwatchMedia(uint64_t tab) {
  Watches().erase(tab);
}

}  // namespace baa

void baa_tab_media_action(BaaTabId tab_id, uint32_t action) {
  content::WebContents* wc = baa::ContentsOf(baa::TabFromId(tab_id));
  if (!wc) {
    return;
  }
  content::MediaSession* session = content::MediaSession::Get(wc);
  switch (action) {
    case BAA_MEDIA_PLAY:
      session->Resume(content::MediaSession::SuspendType::kUI);
      break;
    case BAA_MEDIA_PAUSE:
      session->Suspend(content::MediaSession::SuspendType::kUI);
      break;
    case BAA_MEDIA_NEXT:
      session->NextTrack();
      break;
    case BAA_MEDIA_PREVIOUS:
      session->PreviousTrack();
      break;
    case BAA_MEDIA_PICTURE_IN_PICTURE:
      session->EnterPictureInPicture();
      break;
    case BAA_MEDIA_EXIT_PICTURE_IN_PICTURE:
      // Chrome's exit closes whichever PiP window is open; only this tab's.
      if (wc->HasPictureInPictureVideo()) {
        session->ExitPictureInPicture();
      }
      break;
  }
}
