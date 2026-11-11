// History swipes: Chrome's two-finger swipe handed to the app
// (patches/baa/history-swipe-hook.patch), and a picture of each page a tab
// leaves so the app can show the page a swipe goes to.

#import <Cocoa/Cocoa.h>

#include <cstdint>
#include <list>
#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/time/time.h"
#include "chrome/browser/renderer_host/baa_history_swipe_hook.h"
#include "components/viz/common/frame_sinks/copy_output_result.h"
#include "content/public/browser/navigation_controller.h"
#include "content/public/browser/navigation_entry.h"
#include "content/public/browser/render_widget_host_view.h"
#include "content/public/browser/web_contents.h"
#include "third_party/skia/include/core/SkBitmap.h"
#include "third_party/skia/include/core/SkImageInfo.h"
#include "ui/gfx/geometry/rect.h"

namespace baa {

namespace {

struct Picture {
  uintptr_t contents;  // identity only, never dereferenced
  int entry;           // NavigationEntry::GetUniqueID
  SkBitmap bitmap;
};

// ponytail: the 12 newest pictures over all tabs, at the page's size in points
// (about 8 MB each on a large display). Ceiling: a page further back swipes in
// as a blank sheet. Upgrade: Chrome's own NavigationEntryScreenshotCache, which
// compresses (Android-only plumbing today).
constexpr size_t kMaxPictures = 12;

std::list<Picture>& Pictures() {
  static base::NoDestructor<std::list<Picture>> pictures;
  return *pictures;
}

uintptr_t Key(content::WebContents* contents) {
  return reinterpret_cast<uintptr_t>(contents);
}

void Keep(base::WeakPtr<content::WebContents> contents,
          int entry,
          const content::CopyFromSurfaceResult& result) {
  if (!contents || !result.has_value() || result->bitmap.drawsNothing()) {
    return;
  }
  uintptr_t key = Key(contents.get());
  auto& pictures = Pictures();
  pictures.remove_if([&](const Picture& p) { return p.contents == key && p.entry == entry; });
  pictures.push_front({key, entry, result->bitmap});
  if (pictures.size() > kMaxPictures) {
    pictures.pop_back();
  }
}

void OnSwipe(void* view, int phase, bool forward, double amount) {
  BaaTab* tab = TabForWidgetView(view);
  content::WebContents* contents = ContentsOf(tab);
  if (!contents || !TabAnnounced(tab)) {
    return;
  }
  std::vector<uint8_t> pixels;
  int width = 0, height = 0;
  if (phase == BAA_SWIPE_BEGAN) {
    if (auto* entry = contents->GetController().GetEntryAtOffset(forward ? 1 : -1)) {
      for (const Picture& p : Pictures()) {
        if (p.contents != Key(contents) || p.entry != entry->GetUniqueID()) {
          continue;
        }
        // Whatever Skia gave us, the app reads BGRA premultiplied.
        SkImageInfo info = SkImageInfo::Make(p.bitmap.width(), p.bitmap.height(),
                                             kBGRA_8888_SkColorType, kPremul_SkAlphaType);
        pixels.resize(info.computeMinByteSize());
        if (p.bitmap.readPixels(info, pixels.data(), info.minRowBytes(), 0, 0)) {
          width = info.width();
          height = info.height();
        } else {
          pixels.clear();
        }
        break;
      }
    }
  }
  Callbacks().swipe(CallbackContext(), IdOf(tab), phase, forward, static_cast<float>(amount),
                    pixels.empty() ? nullptr : pixels.data(), width, height);
}

}  // namespace

void KeepPicture(content::WebContents* contents) {
  content::NavigationEntry* entry = contents->GetController().GetLastCommittedEntry();
  content::RenderWidgetHostView* view = contents->GetRenderWidgetHostView();
  if (!entry || !view || !view->IsSurfaceAvailableForCopy()) {
    return;
  }
  // The page as it was left; a hidden tab's last frame is fine too.
  view->CopyFromSurface(gfx::Rect(), view->GetViewBounds().size(), base::Seconds(1),
                        base::BindOnce(&Keep, contents->GetWeakPtr(), entry->GetUniqueID()));
}

void ForgetPictures(content::WebContents* contents) {
  uintptr_t key = Key(contents);
  Pictures().remove_if([&](const Picture& p) { return p.contents == key; });
}

void InstallSwipeHook() {
  g_baa_history_swipe = &OnSwipe;
}

}  // namespace baa
