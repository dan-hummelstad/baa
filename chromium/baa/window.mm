// baa::Window: the BrowserWindow behind every baa Browser.
// PLAN-chromium-phase3.md "baa::Window".

#import <Cocoa/Cocoa.h>

#include <utility>

#include "baa/internal.h"
#include "baa/window.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/task/single_thread_task_runner.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/browser_active_state_manager/browser_active_state_manager.h"
#include "chrome/browser/ui/browser_manager_service.h"
#include "chrome/browser/ui/browser_window/public/browser_collection.h"
#include "chrome/browser/ui/browser_window/public/profile_browser_collection.h"
#include "chrome/browser/ui/browser_window/public/browser_window_features.h"
#include "chrome/browser/ui/find_bar/find_bar.h"
#include "chrome/browser/ui/views/bubble_anchor_util_views.h"
#include "chrome/browser/ui/unload_controller.h"
#include "chrome/browser/ui/extensions/extensions_container.h"
#include "components/input/native_web_keyboard_event.h"
#include "ui/base/unowned_user_data/scoped_unowned_user_data.h"
#include "third_party/blink/public/common/input/web_input_event.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "content/public/browser/keyboard_event_processing_result.h"
#include "content/public/browser/web_contents.h"
#include "ui/base/mojom/window_show_state.mojom.h"
#include "ui/color/color_provider_key.h"
#include "ui/color/color_provider_manager.h"
#include "ui/gfx/geometry/point.h"
#include "ui/gfx/geometry/rect.h"
#include "ui/gfx/geometry/size.h"
#include "ui/gfx/mac/coordinate_conversion.h"
#include "ui/gfx/range/range.h"
#include "ui/gfx/native_ui_types.h"

namespace baa {

namespace {

// The window Chrome treats as active (last-active Browser, "current window"
// for extensions, the foremost tab for JS dialogs). One at a time.
Window* g_active = nullptr;

// Live windows, to find the one of a Browser.
std::vector<Window*>& Windows() {
  static base::NoDestructor<std::vector<Window*>> windows;
  return *windows;
}

// FindBar is never shown (baa finds through FindTabHelper); Chrome only needs
// a non-null object when IDC_FIND runs or a tab with an active find moves.
class StubFindBar : public FindBar {
 public:
  FindBarController* GetFindBarController() const override { return controller_; }
  void SetFindBarController(FindBarController* c) override { controller_ = c; }
  void Show(bool, bool) override {}
  void Hide(bool) override {}
  void SetFocusAndSelection() override {}
  void ClearResults(const find_in_page::FindNotificationDetails&) override {}
  void StopAnimation() override {}
  void MoveWindowIfNecessary() override {}
  void SetFindTextAndSelectedRange(const std::u16string&,
                                   const gfx::Range&) override {}
  std::u16string_view GetFindText() const override { return {}; }
  gfx::Range GetSelectedRange() const override { return gfx::Range(); }
  void UpdateUIForFindResult(const find_in_page::FindNotificationDetails&,
                             const std::u16string&) override {}
  void AudibleAlert() override {}
  bool IsFindBarVisible() const override { return false; }
  void RestoreSavedFocus() override {}
  bool HasGlobalFindPasteboard() const override { return false; }
  void UpdateFindBarForChangedWebContents() override {}
  bool CanPopulateFromSelectedText() override { return false; }
  const FindBarTesting* GetFindBarTesting() const override { return nullptr; }
  bool HasFocus() const override { return false; }
  void CloseOverlappingBubbles() override {}
  views::Widget* GetHostWidget() override { return nullptr; }

 private:
  raw_ptr<FindBarController> controller_ = nullptr;
};

}  // namespace

class Window::StubLocationBar : public LocationBar {
   public:
    StubLocationBar() : LocationBar(/*command_updater=*/nullptr) {}
    StubLocationBar(const StubLocationBar&) = delete;
    StubLocationBar& operator=(const StubLocationBar&) = delete;
    ~StubLocationBar() override = default;

    // LocationBar:
    void FocusLocation(bool select_all, bool clear_focus_if_failed) override {}
    void FocusSearch() override {}
    void UpdateFocusBehavior(bool toolbar_visible) override {}
    void UpdateContentSettingsIcons() override {}
    void SaveStateToContents(content::WebContents* contents) override {}
    void Revert() override {}
    OmniboxView* GetOmniboxView() override;
    OmniboxPopupView* GetOmniboxPopupView() override;
    OmniboxController* GetOmniboxController() override;
    bool ShouldCloseOmniboxPopup(ui::MouseEvent* event) override;
    ChipController* GetChipController() override;
    LocationBarTesting* GetLocationBarForTesting() override;
    LocationBarModel* GetLocationBarModel() override;
    content::WebContents* GetWebContents() override;
    std::optional<bubble_anchor_util::AnchorConfiguration> GetChipAnchor()
        override;
    void OnChanged() override {}
    void UpdateWithoutTabRestore() override {}
    ui::TrackedElement* GetAnchorOrNull() override;
    BrowserWindowInterface* GetBrowser() override;
    Profile* GetProfile() override;
    bool IsInitialized() const override;
    bool IsVisible() const override;
    bool IsDrawn() const override;
    bool IsFullscreen() const override;
    bool IsEditingOrEmpty() const override;
    bool IsMouseHovered() const override;
    bool IsFocusWithin() const override;
    void InvalidateLayout() override {}
    gfx::Rect Bounds() const override;
    gfx::Rect BoundsInScreen() const override;
    gfx::Size MinimumSize() const override;
    gfx::Size PreferredSize() const override;
    void Update(content::WebContents* contents) override {}
    void ResetTabState(content::WebContents* contents) override {}
    bool HasSecurityStateChanged() override;
    void AnnounceAlert(const std::u16string& announcement) override {}
};

OmniboxView* Window::StubLocationBar::GetOmniboxView() {
  return nullptr;
}

OmniboxPopupView* Window::StubLocationBar::GetOmniboxPopupView() {
  return nullptr;
}

OmniboxController* Window::StubLocationBar::GetOmniboxController() {
  return nullptr;
}

bool Window::StubLocationBar::ShouldCloseOmniboxPopup(
    ui::MouseEvent* event) {
  return false;
}

ChipController* Window::StubLocationBar::GetChipController() {
  return nullptr;
}

LocationBarTesting*
Window::StubLocationBar::GetLocationBarForTesting() {
  return nullptr;
}

LocationBarModel* Window::StubLocationBar::GetLocationBarModel() {
  return nullptr;
}

content::WebContents* Window::StubLocationBar::GetWebContents() {
  return nullptr;
}

std::optional<bubble_anchor_util::AnchorConfiguration>
Window::StubLocationBar::GetChipAnchor() {
  return {};
}

ui::TrackedElement* Window::StubLocationBar::GetAnchorOrNull() {
  return nullptr;
}

BrowserWindowInterface* Window::StubLocationBar::GetBrowser() {
  return nullptr;
}

Profile* Window::StubLocationBar::GetProfile() {
  return nullptr;
}

bool Window::StubLocationBar::IsInitialized() const {
  return true;
}

bool Window::StubLocationBar::IsVisible() const {
  return true;
}

bool Window::StubLocationBar::IsDrawn() const {
  return true;
}

bool Window::StubLocationBar::IsFullscreen() const {
  return false;
}

bool Window::StubLocationBar::IsEditingOrEmpty() const {
  return false;
}

bool Window::StubLocationBar::IsMouseHovered() const {
  return false;
}

bool Window::StubLocationBar::IsFocusWithin() const {
  return false;
}

gfx::Rect Window::StubLocationBar::Bounds() const {
  return gfx::Rect();
}

gfx::Rect Window::StubLocationBar::BoundsInScreen() const {
  return gfx::Rect();
}

gfx::Size Window::StubLocationBar::MinimumSize() const {
  return gfx::Size();
}

gfx::Size Window::StubLocationBar::PreferredSize() const {
  return gfx::Size();
}

bool Window::StubLocationBar::HasSecurityStateChanged() {
  return false;
}
ui::ZOrderLevel Window::GetZOrderLevel() const {
  return ui::ZOrderLevel::kNormal;
}

bool Window::IsOnCurrentWorkspace() const {
  return true;
}

bool Window::IsVisibleOnScreen() const {
  return true;
}

void Window::SetTopControlsShownRatio(
    content::WebContents* web_contents,
    float ratio) {}

bool Window::DoBrowserControlsShrinkRendererSize(
    const content::WebContents* contents) const {
  return false;
}

ui::NativeTheme* Window::GetNativeTheme() {
  return nullptr;
}

const ui::ThemeProvider* Window::GetThemeProvider() const {
  return nullptr;
}

const ui::ColorProvider* Window::GetColorProvider() const {
  return ui::ColorProviderManager::Get().GetColorProviderFor(
      ui::ColorProviderKey());
}

int Window::GetTopControlsHeight() const {
  return 0;
}

void Window::SetTopControlsGestureScrollInProgress(
    bool in_progress) {}

std::vector<StatusBubble*> Window::GetStatusBubbles() {
  return {};
}

gfx::Rect Window::GetRestoredBounds() const {
  return gfx::Rect();
}

ui::mojom::WindowShowState Window::GetRestoredState() const {
  return ui::mojom::WindowShowState::kDefault;
}

gfx::Size Window::GetContentsSize() const {
  return gfx::Size();
}

void Window::SetContentsSize(const gfx::Size& size) {}

bool Window::IsMaximized() const {
  return false;
}

bool Window::IsMinimized() const {
  return false;
}

bool Window::GetCanResize() {
  return false;
}

ui::mojom::WindowShowState Window::GetWindowShowState() const {
  return ui::mojom::WindowShowState::kDefault;
}

bool Window::IsFullscreen() const {
  return false;
}

bool Window::UpdateToolbarSecurityState() {
  return false;
}

ui::AcceleratorProvider* Window::GetAcceleratorProvider() {
  return nullptr;
}

PinnedToolbarActions* Window::GetPinnedToolbarActions() {
  return nullptr;
}

autofill::AutofillBubbleHandler* Window::GetAutofillBubbleHandler() {
  return nullptr;
}

bool Window::IsToolbarVisible() const {
  return false;
}

bool Window::IsToolbarShowing() const {
  return false;
}

bool Window::IsLocationBarVisible() const {
  return false;
}

bool Window::IsUnframedModeEnabled() const {
  return false;
}

BrowserView* Window::AsBrowserView() {
  return nullptr;
}

ShowTranslateBubbleResult Window::ShowTranslateBubble(
    content::WebContents* contents,
    translate::TranslateStep step,
    const std::string& source_language,
    const std::string& target_language,
    translate::TranslateErrors error_type,
    bool is_user_gesture) {
  return ShowTranslateBubbleResult::kSuccess;
}

DownloadBubbleUIController* Window::GetDownloadBubbleUIController() {
  return nullptr;
}

std::string Window::GetWorkspace() const {
  return std::string();
}

bool Window::IsVisibleOnAllWorkspaces() const {
  return false;
}

std::unique_ptr<content::EyeDropper> Window::OpenEyeDropper(
    content::RenderFrameHost* frame,
    content::EyeDropperListener* listener) {
  return nullptr;
}

Window::Window(BrowserWindowInterface* browser, bool hidden)
    : browser_(browser), location_bar_(std::make_unique<StubLocationBar>()), hidden_(hidden) {
  // Without one registered, extension context menus and the CDP extensions
  // handler dereference null.
  // A close the user cancelled (beforeunload "Stay") leaves the window usable and closable again.
  close_cancelled_ = browser->RegisterBrowserCloseCancelled(
      base::BindRepeating([](Window* self, BrowserWindowInterface*, BrowserWindowInterface::ClosingStatus) { self->closing_ = false; },
                          base::Unretained(this)));
  extensions_container_ = MakeExtensionsContainer(browser);
  extensions_registration_ = std::make_unique<ui::ScopedUnownedUserData<ExtensionsContainer>>(
      browser->GetUnownedUserDataHost(), *extensions_container_);
  // The Browser's TabStripModel exists already (browser.cc builds it before
  // the window), so nothing is missed.
  browser->GetTabStripModel()->AddObserver(this);
  Windows().push_back(this);
}

Window::~Window() {
  for (auto& observer : observers_) {
    observer.OnHostDestroying();
  }
  if (g_active == this) {
    g_active = nullptr;
  }
  std::erase(Windows(), this);
  // BrowserWindow implementations are expected to call this before destruction.
  browser_->GetFeatures().TearDownPreBrowserWindowDestruction();
}

// static
BrowserWindow* Window::Create(BrowserWindowInterface* browser,
                              bool user_gesture) {
  switch (browser->GetType()) {
    case BrowserWindowInterface::TYPE_NORMAL:
    case BrowserWindowInterface::TYPE_POPUP:
      return new Window(browser);
    case BrowserWindowInterface::TYPE_DEVTOOLS:
      // Docked: the app shows the front-end. Otherwise Chrome's views window.
      return DevtoolsDocking() ? new Window(browser, /*hidden=*/true) : nullptr;
    default:
      return nullptr;
  }
}

bool IsOurBrowser(BrowserWindowInterface* browser) {
  for (Window* window : Windows()) {
    if (window->browser() == browser) {
      return true;
    }
  }
  return false;
}

bool BrowserIsClosing(BrowserWindowInterface* browser) {
  for (Window* window : Windows()) {
    if (window->browser() == browser) {
      return window->closing();
    }
  }
  return true;  // not ours any more
}

void DestroyBrowsersOf(Profile* profile) {
  std::vector<BrowserWindowInterface*> doomed;
  // Ours, then Chrome's own (a DevTools window, a popup): every Browser of the
  // profile must be gone before an OTR profile is destroyed.
  if (auto* collection = ProfileBrowserCollection::GetForProfile(profile)) {
    collection->ForEach(
        [&](BrowserWindowInterface* browser) {
          doomed.push_back(browser);
          return true;
        },
        BrowserCollection::Order::kCreation);
  }
  for (Window* window : Windows()) {
    if (window->browser()->GetProfile() == profile &&
        std::find(doomed.begin(), doomed.end(), window->browser()) == doomed.end()) {
      doomed.push_back(window->browser());
    }
  }
  for (BrowserWindowInterface* browser : doomed) {
    // Tabs first: the Browser must be empty to be destroyed.
    TabStripModel* model = browser->GetTabStripModel();
    while (model->count() > 0) {
      model->DetachAndDeleteWebContentsAt(model->count() - 1);
    }
    BrowserManagerService::SynchronouslyDestroyBrowser(browser);
  }
}

void ActivateBrowser(BrowserWindowInterface* browser) {
  for (Window* window : Windows()) {
    if (window->browser() == browser) {
      window->Activate();
      return;
    }
  }
}

void Window::DeleteBrowserWindow() {
  delete this;
}

// Browser::TabStripEmpty calls this from inside a tab strip notification, and
// UnloadController closes tabs and schedules the Browser's deletion; neither
// may run in that stack, so it is posted. Calls after the first are no-ops.
void Window::Close() {
  if (closing_) {
    return;
  }
  closing_ = true;
  base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
      FROM_HERE,
      base::BindOnce(
          [](base::WeakPtr<Window> window) {
            if (!window) {
              return;
            }
            UnloadController::From(window->browser_)->OnWindowClosing();
          },
          weak_factory_.GetWeakPtr()));
}

bool Window::IsVisible() const {
  return true;
}

void Window::Activate() {
  if (hidden_) {
    return;  // extensions' "current window" must stay the page's
  }
  if (g_active == this) {
    return;
  }
  if (g_active) {
    Window* previous = std::exchange(g_active, nullptr);
    BrowserActiveStateManager::From(previous->browser_)->DidBecomeInactive();
  }
  g_active = this;
  BrowserActiveStateManager::From(browser_)->DidBecomeActive();
}

void Window::Deactivate() {
  if (g_active == this) {
    g_active = nullptr;
    BrowserActiveStateManager::From(browser_)->DidBecomeInactive();
  }
}

bool Window::IsActive() const {
  return g_active == this;
}

bool Window::IsTabStripEditable() const {
  return true;
}
void Window::DisableTabStripEditingForTesting() {}

NSView* Window::ActiveView() const {
  content::WebContents* contents =
      browser_->GetTabStripModel()->GetActiveWebContents();
  return contents ? contents->GetNativeView().GetNativeNSView() : nil;
}

gfx::NativeWindow Window::GetNativeWindow() const {
  return gfx::NativeWindow(ActiveView().window);
}

gfx::Rect Window::GetBounds() const {
  NSWindow* window = ActiveView().window;
  return window ? gfx::ScreenRectFromNSRect(window.frame) : gfx::Rect();
}

LocationBar* Window::GetLocationBar() const {
  return location_bar_.get();
}

std::unique_ptr<FindBar> Window::CreateFindBar() {
  return std::make_unique<StubFindBar>();
}

void Window::ConfirmBrowserCloseWithPendingDownloads(
    int,
    UnloadController::DownloadCloseType,
    base::OnceCallback<void(bool)> callback) {
  // Never asking: the UnloadController waits on this for a close to finish.
  std::move(callback).Run(true);
}

// The app's shortcuts get the first look, as with CEF's on_pre_key_event.
content::KeyboardEventProcessingResult Window::PreHandleKeyboardEvent(
    const input::NativeWebKeyboardEvent& event) {
  return OfferKey(event, false)
             ? content::KeyboardEventProcessingResult::HANDLED
             : content::KeyboardEventProcessingResult::NOT_HANDLED;
}

bool Window::HandleKeyboardEvent(const input::NativeWebKeyboardEvent& event) {
  return OfferKey(event, true);
}

web_modal::WebContentsModalDialogHost* Window::GetWebContentsModalDialogHost() {
  return this;
}

web_modal::WebContentsModalDialogHost* Window::GetWebContentsModalDialogHostFor(
    content::WebContents*) {
  return this;
}

// With a null host every tab-modal views dialog CHECKs; with this one they
// show as loose windows until M2 intercepts each.
gfx::NativeView Window::GetHostView() const {
  return gfx::NativeView(ActiveView());
}

// The views dialogs of M2 (HTTP auth, install prompts) become top-level
// windows that are not children of ours, so the position is in screen
// coordinates: centred at the top of the page.
gfx::Point Window::GetDialogPosition(const gfx::Size& size) {
  NSView* view = ActiveView();
  if (!view || !view.window) {
    return gfx::Point();
  }
  NSRect in_window = [view convertRect:view.bounds toView:nil];
  gfx::Rect screen = gfx::ScreenRectFromNSRect([view.window convertRectToScreen:in_window]);
  return gfx::Point(screen.x() + std::max(0, (screen.width() - size.width()) / 2), screen.y());
}

gfx::Size Window::GetMaximumDialogSize() {
  NSView* view = ActiveView();
  return view ? gfx::Size(view.bounds.size.width, view.bounds.size.height)
              : gfx::Size();
}

void Window::AddObserver(web_modal::ModalDialogHostObserver* observer) {
  observers_.AddObserver(observer);
}

void Window::RemoveObserver(web_modal::ModalDialogHostObserver* observer) {
  observers_.RemoveObserver(observer);
}

// ponytail: page fullscreen is M2 (toggle the gpui window, then
// fullscreen_controller()->WindowFullscreenStateChanged()).
ExclusiveAccessContext* Window::GetExclusiveAccessContext() {
  return this;
}
Profile* Window::GetProfile() {
  return browser_->GetProfile();
}
void Window::EnterFullscreen(const url::Origin&,
                             ExclusiveAccessBubbleType,
                             FullscreenTabParams) {}
void Window::ExitFullscreen() {}
void Window::UpdateExclusiveAccessBubble(
    const ExclusiveAccessBubbleParams&,
    ExclusiveAccessBubbleHideCallback first_hide_callback) {
  if (first_hide_callback) {
    std::move(first_hide_callback)
        .Run(ExclusiveAccessBubbleHideReason::kInterrupted);
  }
}
bool Window::IsExclusiveAccessBubbleDisplayed() const {
  return false;
}
void Window::OnExclusiveAccessUserInput() {}
content::WebContents* Window::GetWebContentsForExclusiveAccess() {
  return browser_->GetTabStripModel()->GetActiveWebContents();
}
bool Window::CanUserEnterFullscreen() const {
  return true;
}
bool Window::CanUserExitFullscreen() const {
  return true;
}

void Window::OnTabStripModelChanged(TabStripModel* model,
                                    const TabStripModelChange& change,
                                    const TabStripSelectionChange& selection) {
  switch (change.type()) {
    case TabStripModelChange::kInserted:
      for (const auto& inserted : change.GetInsert()->contents) {
        OnTabInserted(model, inserted.tab, inserted.index,
                      /*background=*/inserted.contents !=
                          model->GetActiveWebContents());
      }
      break;
    case TabStripModelChange::kRemoved:
      for (const auto& removed : change.GetRemove()->contents) {
        OnTabRemoved(removed.tab, removed.remove_reason);
      }
      break;
    case TabStripModelChange::kReplaced: {
      const auto* replace = change.GetReplace();
      OnTabReplaced(replace->tab, replace->new_contents);
      break;
    }
    default:
      break;
  }
}


// The app's shortcuts get the first look, as with CEF's on_pre_key_event;
// Escape too, so an overlay over the page can close.
bool OfferKey(const input::NativeWebKeyboardEvent& event, bool after_page) {
  if (event.GetType() != blink::WebInputEvent::Type::kRawKeyDown) {
    return false;
  }
  const int shortcut =
      blink::WebInputEvent::kMetaKey | blink::WebInputEvent::kControlKey;
  if (!(event.GetModifiers() & shortcut) &&
      event.windows_key_code != ::ui::VKEY_ESCAPE) {
    return false;
  }
  NSEvent* ns_event = event.os_event.Get();
  if (!ns_event) {
    return false;
  }
  return Callbacks().key(CallbackContext(), (__bridge void*)ns_event,
                         after_page);
}

}  // namespace baa
