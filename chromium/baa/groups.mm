// chrome.tabGroups <-> baa folders (PLAN-m3.md 2.4). A group's key is the app's
// folder key, or NextId() | 1<<63 for a group something else made.
// The app's groups are ephemeral: Chrome's saved-tab-group service skips them,
// so nothing piles up in the profile across launches and no BrowserView is
// reached for. A group an extension makes is marked ephemeral when first seen
// here, which is before the service's own OnTabGroupAdded runs.

#include <algorithm>
#include <optional>
#include <string>
#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "baa/window.h"
#include "base/no_destructor.h"
#include "base/strings/utf_string_conversions.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/tab_group_model.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "components/tab_groups/tab_group_color.h"
#include "components/tab_groups/tab_group_id.h"
#include "components/tab_groups/tab_group_visual_data.h"
#include "components/tabs/public/tab_group.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/web_contents.h"

namespace baa {
namespace {

constexpr uint64_t kChromeKey = 1ull << 63;

// A key can have a group per Browser (a folder with tabs in two profiles).
struct Entry {
  uint64_t key;
  tab_groups::TabGroupId id;
};

std::vector<Entry>& Entries() {
  static base::NoDestructor<std::vector<Entry>> entries;
  return *entries;
}

// Set while baa_tab_set_group runs: what Chrome reports meanwhile is our own echo.
bool g_applying = false;

std::optional<Entry> ByKey(uint64_t key, TabStripModel* model) {
  for (const Entry& e : Entries()) {
    if (e.key == key && model->group_model()->ContainsTabGroup(e.id)) {
      return e;
    }
  }
  return std::nullopt;
}

std::optional<Entry> ById(const tab_groups::TabGroupId& id) {
  for (const Entry& e : Entries()) {
    if (e.id == id) {
      return e;
    }
  }
  return std::nullopt;
}

std::string TitleOf(TabStripModel* model, const tab_groups::TabGroupId& id) {
  if (!model->group_model()->ContainsTabGroup(id)) {
    return "";
  }
  return base::UTF16ToUTF8(
      model->group_model()->GetTabGroup(id)->visual_data()->title());
}

void Send(tabs::TabInterface* tab, uint64_t key, const std::string& title) {
  content::WebContents* contents = tab->GetContents();
  BaaTab* t = contents ? TabFor(contents) : nullptr;
  if (t && TabAnnounced(t) && TabAlive(t)) {
    Callbacks().tab_group(CallbackContext(), IdOf(t), key, title.c_str());
  }
}

}  // namespace

void GroupsTabGrouped(TabStripModel* model,
                      std::optional<tab_groups::TabGroupId> old_group,
                      std::optional<tab_groups::TabGroupId> new_group,
                      tabs::TabInterface* tab) {
  if (g_applying || !model->SupportsTabGroups()) {
    return;
  }
  if (!new_group) {
    // Only a live tab being ungrouped counts. A tab that closed, was unloaded or
    // moved to another strip is already out of the model, and the app must keep
    // it in its folder.
    if (old_group && model->GetIndexOfTab(tab) != TabStripModel::kNoTab) {
      Send(tab, 0, "");
    }
    return;
  }
  std::optional<Entry> e = ById(*new_group);
  if (!e) {
    model->group_model()->GetTabGroup(*new_group)->SetIsEphemeral(true);
    e = Entry{NextId() | kChromeKey, *new_group};
    Entries().push_back(*e);
  }
  Send(tab, e->key, TitleOf(model, *new_group));
}

void GroupsChanged(const TabGroupChange& change) {
  TabStripModel* model = change.model;
  if (change.type == TabGroupChange::kClosed) {
    const auto* close = change.GetCloseChange();
    // A group that moves to another of our strips keeps its key.
    if (close && close->reason() ==
                     TabGroupChange::TabGroupClosureReason::kDetachedToAnotherTabstrip) {
      return;
    }
    std::erase_if(Entries(),
                  [&](const Entry& e) { return e.id == change.group; });
    return;
  }
  if (g_applying || change.type != TabGroupChange::kVisualsChanged) {
    return;
  }
  std::optional<Entry> e = ById(change.group);
  const auto* visuals = change.GetVisualsChange();
  // The empty change after a group's first tab carries nothing GroupsTabGrouped
  // has not sent already.
  if (!e || !visuals->new_visuals ||
      !model->group_model()->ContainsTabGroup(change.group) ||
      (visuals->old_visuals &&
       visuals->old_visuals->title() == visuals->new_visuals->title())) {
    return;  // not ours to report, or only the color changed
  }
  const std::string title = base::UTF16ToUTF8(visuals->new_visuals->title());
  gfx::Range range = model->group_model()->GetTabGroup(change.group)->ListTabs();
  for (size_t i = range.start(); i < range.end(); ++i) {
    if (tabs::TabInterface* tab = model->GetTabAtIndex(i)) {
      Send(tab, e->key, title);
    }
  }
}

}  // namespace baa

void baa_tab_set_group(BaaTabId tab_id, uint64_t key, const char* title_c) {
  tabs::TabInterface* ti = baa::TabInterfaceOf(baa::TabFromId(tab_id));
  BrowserWindowInterface* browser = ti ? ti->GetBrowserWindowInterface() : nullptr;
  TabStripModel* model = browser ? browser->GetTabStripModel() : nullptr;
  int index = model ? model->GetIndexOfTab(ti) : TabStripModel::kNoTab;
  if (index == TabStripModel::kNoTab || !model->SupportsTabGroups()) {
    return;
  }
  const std::u16string title = base::UTF8ToUTF16(title_c ? title_c : "");
  const std::optional<tab_groups::TabGroupId> current = ti->GetGroup();
  baa::g_applying = true;
  if (key == 0) {
    if (current) {
      model->RemoveFromGroup({index});
    }
  } else {
    std::optional<baa::Entry> known = baa::ByKey(key, model);
    tab_groups::TabGroupId id = known ? known->id : tab_groups::TabGroupId::GenerateNew();
    if (known) {
      if (current != id) {
        model->AddToExistingGroup({index}, id);
      }
    } else {
      id = model->AddToNewGroup({index}, /*is_ephemeral=*/true);
      baa::Entries().push_back({key, id});
    }
    const tab_groups::TabGroupVisualData* old =
        model->group_model()->GetTabGroup(id)->visual_data();
    if (!known || (!title.empty() && old->title() != title)) {
      model->ChangeTabGroupVisuals(
          id, tab_groups::TabGroupVisualData(
                  title.empty() ? old->title() : title,
                  tab_groups::TabGroupColorId::kGrey, old->is_collapsed()));
    }
  }
  baa::g_applying = false;
}
