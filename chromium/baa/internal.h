// Shared declarations for //baa. Spec: PLAN-chromium-phase3.md "C++ layout".
#ifndef BAA_INTERNAL_H_
#define BAA_INTERNAL_H_

#include <memory>
#include <optional>
#include <string>
#include <vector>

#include "baa/baa.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/observer_list.h"
#include "base/scoped_observation.h"
#include "build/build_config.h"
#include "chrome/browser/ui/browser_window.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/dialogs/browser_dialogs.h"
#include "chrome/browser/ui/exclusive_access/exclusive_access_context.h"
#include "chrome/browser/ui/location_bar/location_bar.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/tabs/tab_strip_model_observer.h"
#include "chrome/browser/ui/translate/partial_translate_bubble_model.h"
#include "chrome/browser/ui/unload_controller.h"
#include "chrome/common/buildflags.h"
#include "components/apps/link_capturing/intent_picker_info.h"
#include "components/user_education/common/new_badge/new_badge_controller.h"
#include "components/web_modal/web_contents_modal_dialog_host.h"
#include "content/public/browser/web_contents.h"
#include "ui/base/mojom/window_show_state.mojom-forward.h"

@class NSView;

class LocationBarTesting;
class OmniboxView;

namespace baa {

// The callbacks from baa_install_app; zeroed until then.
const BaaCallbacks& Callbacks();
void* CallbackContext();
// True once Chrome began terminating: no new Browsers or tabs, no new work.
bool Terminating();
void WatchTermination();

// window.mm -> tab.mm: what the window's tab strip observer found.
void OnTabInserted(TabStripModel* model,
                   tabs::TabInterface* tab,
                   int index,
                   bool background);
void OnTabRemoved(tabs::TabInterface* tab, TabRemovedReason reason);
void OnTabReplaced(tabs::TabInterface* tab, content::WebContents* new_contents);
// Reports a key to the app. Synchronous.
bool OfferKey(const input::NativeWebKeyboardEvent& event, bool after_page);


// Makes `browser`'s window the one Chrome treats as active.
void ActivateBrowser(BrowserWindowInterface* browser);

// The BrowserWindow of every baa Browser: Chrome's logic with no views behind
// it, drawn by the embedding app. The body is TestBrowserWindow's long tail of
// no-ops plus the getters Chrome dereferences (PLAN-chromium-phase3.md 15a).
class Window : public BrowserWindow,
               public TabStripModelObserver,
               public web_modal::WebContentsModalDialogHost,
               public ExclusiveAccessContext {
 public:
  explicit Window(BrowserWindowInterface* browser);
  Window(const Window&) = delete;
  Window& operator=(const Window&) = delete;
  ~Window() override;

  // Null for Browser types that keep Chrome's views windows (DevTools, PiP, apps).
  static BrowserWindow* Create(BrowserWindowInterface* browser,
                               bool user_gesture);

  BrowserWindowInterface* browser() const { return browser_; }

  // BrowserWindow:
  void Show() override {}
  void ShowInactive() override {}
  void Hide() override {}
  bool IsVisible() const override;
  void SetBounds(const gfx::Rect& bounds) override {}
  void Close() override;
  void Activate() override;
  void Deactivate() override;
  bool IsActive() const override;
  void FlashFrame(bool flash) override {}
  ui::ZOrderLevel GetZOrderLevel() const override;
  void SetZOrderLevel(ui::ZOrderLevel order) override {}
  gfx::NativeWindow GetNativeWindow() const override;
  bool IsOnCurrentWorkspace() const override;
  bool IsVisibleOnScreen() const override;
  void SetTopControlsShownRatio(content::WebContents* web_contents,
                                float ratio) override;
  bool DoBrowserControlsShrinkRendererSize(
      const content::WebContents* contents) const override;
  ui::NativeTheme* GetNativeTheme() override;
  const ui::ThemeProvider* GetThemeProvider() const override;
  const ui::ColorProvider* GetColorProvider() const override;
  int GetTopControlsHeight() const override;
  void SetTopControlsGestureScrollInProgress(bool in_progress) override;
  std::vector<StatusBubble*> GetStatusBubbles() override;
  void UpdateTitleBar() override {}
  void UpdateLoadingAnimations(bool is_visible) override {}
  void OnActiveTabChanged(content::WebContents* old_contents,
                          content::WebContents* new_contents,
                          int index,
                          int reason) override {}
  void OnTabDetached(content::WebContents* contents, bool was_active) override {
  }

  gfx::Rect GetRestoredBounds() const override;
  ui::mojom::WindowShowState GetRestoredState() const override;
  gfx::Rect GetBounds() const override;
  gfx::Size GetContentsSize() const override;
  void SetContentsSize(const gfx::Size& size) override;
  bool IsMaximized() const override;
  bool IsMinimized() const override;
  void Maximize() override {}
  void Minimize() override {}
  void Restore() override {}
  bool GetCanResize() override;
  ui::mojom::WindowShowState GetWindowShowState() const override;
  bool IsFullscreen() const override;
  LocationBar* GetLocationBar() const override;
  ui::AcceleratorProvider* GetAcceleratorProvider() override;
  PinnedToolbarActions* GetPinnedToolbarActions() override;
  autofill::AutofillBubbleHandler* GetAutofillBubbleHandler() override;
  void SetFocusToLocationBar(bool is_user_initiated) override {}
  void UpdateReloadStopState(bool is_loading, bool force) override {}
  void UpdateToolbar(content::WebContents* contents) override {}
  bool UpdateToolbarSecurityState() override;
  void UpdateCustomTabBarVisibility(bool visible, bool animate) override {}
  void ResetToolbarTabState(content::WebContents* contents) override {}
  void FocusToolbar() override {}
  void ToolbarSizeChanged(bool is_animating) override {}
  void TabDraggingStatusChanged(bool is_dragging) override {}
  void LinkOpeningFromGesture(WindowOpenDisposition disposition) override {}
  void FocusAppMenu() override {}
  void ShowAppMenu() override {}
  void PreHandleDragUpdate(const content::DropData& drop_data,
                           const gfx::PointF& point) override {}

  void PreHandleDragExit() override {}
  void HandleDragEnded() override {}
  content::KeyboardEventProcessingResult PreHandleKeyboardEvent(
      const input::NativeWebKeyboardEvent& event) override;
  bool HandleKeyboardEvent(const input::NativeWebKeyboardEvent& event) override;
  bool IsTabStripEditable() const override;
  void DisableTabStripEditingForTesting() override;
  void SetIsTabStripEditable(bool is_editable);
  bool IsToolbarVisible() const override;
  bool IsLocationBarVisible() const override;
  bool IsToolbarShowing() const override;
  bool IsUnframedModeEnabled() const override;
  void ShowChromeLabs() override {}
  BrowserView* AsBrowserView() override;
  void ShowUpdateChromeDialog() override {}
  void ShowBookmarkBubble(const GURL& url, bool already_bookmarked) override {}
#if !BUILDFLAG(IS_ANDROID)
  void ShowIntentPickerBubble(
      std::vector<apps::IntentPickerAppInfo> app_info,
      bool show_stay_in_chrome,
      bool show_remember_selection,
      apps::IntentPickerBubbleType bubble_type,
      const std::optional<url::Origin>& initiating_origin,
      IntentPickerResponse callback) override {}
#endif  //  !define(OS_ANDROID)
  ShowTranslateBubbleResult ShowTranslateBubble(
      content::WebContents* contents,
      translate::TranslateStep step,
      const std::string& source_language,
      const std::string& target_language,
      translate::TranslateErrors error_type,
      bool is_user_gesture) override;

  DownloadBubbleUIController* GetDownloadBubbleUIController() override;
  void ConfirmBrowserCloseWithPendingDownloads(
      int download_count,
      UnloadController::DownloadCloseType dialog_type,
      base::OnceCallback<void(bool)> callback) override;

  std::unique_ptr<FindBar> CreateFindBar() override;
  web_modal::WebContentsModalDialogHost* GetWebContentsModalDialogHost()
      override;
  web_modal::WebContentsModalDialogHost* GetWebContentsModalDialogHostFor(
      content::WebContents* web_contents) override;
  void ShowAvatarBubbleFromAvatarButton(bool is_source_keyboard) override {}
  void MaybeShowProfileSwitchIPH() override {}
  void MaybeShowSupervisedUserProfileSignInIPH() override {}

#if BUILDFLAG(IS_CHROMEOS) || BUILDFLAG(IS_MAC) || BUILDFLAG(IS_WIN) || \
    BUILDFLAG(IS_LINUX)
  void ShowHatsDialog(
      const std::string& site_id,
      const std::optional<std::string>& hats_histogram_name,
      const std::optional<uint64_t> hats_survey_ukm_id,
      base::OnceClosure success_callback,
      base::OnceClosure failure_callback,
      const SurveyBitsData& product_specific_bits_data,
      const SurveyStringData& product_specific_string_data) override {}

  void ShowIncognitoClearBrowsingDataDialog() override {}
  void ShowIncognitoHistoryDisclaimerDialog() override {}
#endif

  ExclusiveAccessContext* GetExclusiveAccessContext() override;
  std::string GetWorkspace() const override;
  bool IsVisibleOnAllWorkspaces() const override;
  void ShowEmojiPanel() override {}
  void ShowCaretBrowsingDialog() override {}
  std::unique_ptr<content::EyeDropper> OpenEyeDropper(
      content::RenderFrameHost* frame,
      content::EyeDropperListener* listener) override;


  void CreateTabSearchBubble() override {}
  void CloseTabSearchBubble() override {}

  // TabStripModelObserver:
  void OnTabStripModelChanged(
      TabStripModel* tab_strip_model,
      const TabStripModelChange& change,
      const TabStripSelectionChange& selection) override;

  // web_modal::WebContentsModalDialogHost:
  gfx::NativeView GetHostView() const override;
  gfx::Point GetDialogPosition(const gfx::Size& size) override;
  gfx::Size GetMaximumDialogSize() override;
  void AddObserver(web_modal::ModalDialogHostObserver* observer) override;
  void RemoveObserver(web_modal::ModalDialogHostObserver* observer) override;

  // ExclusiveAccessContext (IsFullscreen is shared with BaseWindow):
  Profile* GetProfile() override;
  void EnterFullscreen(const url::Origin& origin,
                       ExclusiveAccessBubbleType bubble_type,
                       FullscreenTabParams fullscreen_tab_params) override;
  void ExitFullscreen() override;
  void UpdateExclusiveAccessBubble(
      const ExclusiveAccessBubbleParams& params,
      ExclusiveAccessBubbleHideCallback first_hide_callback) override;
  bool IsExclusiveAccessBubbleDisplayed() const override;
  void OnExclusiveAccessUserInput() override;
  content::WebContents* GetWebContentsForExclusiveAccess() override;
  bool CanUserEnterFullscreen() const override;
  bool CanUserExitFullscreen() const override;

 protected:
  void DeleteBrowserWindow() final;

 private:
  class StubLocationBar;
  NSView* ActiveView() const;
  void CloseNow();

  raw_ptr<BrowserWindowInterface> browser_;
  std::unique_ptr<StubLocationBar> location_bar_;
  base::ObserverList<web_modal::ModalDialogHostObserver> observers_;
  bool close_scheduled_ = false;
  base::WeakPtrFactory<Window> weak_factory_{this};
};

}  // namespace baa

#endif  // BAA_INTERNAL_H_
