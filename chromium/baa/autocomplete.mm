// baa_profile_autocomplete: Chrome's AutocompleteController for the command bar
// (PLAN-m3.md §2.5), modelled on OmniboxPageHandler::CreateController.

#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "chrome/browser/autocomplete/chrome_autocomplete_provider_client.h"
#include "chrome/browser/autocomplete/chrome_autocomplete_scheme_classifier.h"
#include "chrome/browser/profiles/profile.h"
#include "components/omnibox/browser/autocomplete_controller.h"
#include "components/omnibox/browser/autocomplete_controller_config.h"
#include "components/omnibox/browser/autocomplete_input.h"
#include "components/omnibox/browser/autocomplete_match.h"
#include "components/omnibox/browser/autocomplete_provider.h"
#include "components/omnibox/browser/autocomplete_result.h"

namespace baa {

struct Autocomplete::Impl : AutocompleteController::Observer {
  explicit Impl(Profile* profile) : profile(profile) {
    // Only these providers: no zero-suggest, document, featured-search or
    // open-tab ones. SearchProvider asks the network only when the profile's
    // search.suggest_enabled is true (BaseSearchProvider::CanSendSuggestRequest)
    // and never for an off-the-record profile. CalculatorProvider only keeps the
    // calculator answers of those suggestions: it computes nothing itself.
    controller = std::make_unique<AutocompleteController>(
        std::make_unique<ChromeAutocompleteProviderClient>(profile),
        AutocompleteControllerConfig{
            .provider_types =
                static_cast<int>(AutocompleteProvider::Type::kHistoryQuick) |
                static_cast<int>(AutocompleteProvider::Type::kHistoryUrl) |
                static_cast<int>(AutocompleteProvider::Type::kSearch) |
                static_cast<int>(AutocompleteProvider::Type::kCalculator),
            .show_iph_matches = false});
    controller->AddObserver(this);
  }
  ~Impl() override { controller->RemoveObserver(this); }

  // Providers answer inside Start, but the app is never called inside its own call.
  void OnResultChanged(AutocompleteController*, bool) override { Post(false); }
  // The stop timer keeps the results without another OnResultChanged.
  void OnAutocompleteStopTimerTriggered(const AutocompleteInput&) override { Post(true); }

  void Post(bool force_done) {
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(&Impl::Send, weak.GetWeakPtr(), id, force_done));
  }

  void Send(uint64_t for_id, bool force_done) {
    if (for_id != id || id == 0) {
      return;  // a newer query (or "") replaced it
    }
    std::vector<std::string> strings;  // three per match; keeps the c strings alive
    std::vector<bool> search;
    for (const AutocompleteMatch& m : controller->result()) {
      if (m.type == omnibox::AutocompleteMatchType::kSearchWhatYouTyped ||
          m.type == omnibox::AutocompleteMatchType::kUrlWhatYouTyped) {
        continue;  // baa's first row covers them
      }
      strings.push_back(m.destination_url.spec());
      strings.push_back(base::UTF16ToUTF8(m.contents));
      strings.push_back(base::UTF16ToUTF8(m.description));
      search.push_back(AutocompleteMatch::IsSearchType(m.type));
    }
    std::vector<BaaMatch> out;
    for (size_t i = 0; i < search.size(); i++) {
      out.push_back({strings[3 * i].c_str(), strings[3 * i + 1].c_str(),
                     strings[3 * i + 2].c_str(), search[i]});
    }
    Callbacks().autocomplete(CallbackContext(), id, out.data(), out.size(),
                             force_done || controller->done());
  }

  raw_ptr<Profile> profile;
  uint64_t id = 0;
  std::unique_ptr<AutocompleteController> controller;
  base::WeakPtrFactory<Impl> weak{this};
};

Autocomplete::Autocomplete(Profile* profile) : impl_(std::make_unique<Impl>(profile)) {}
Autocomplete::~Autocomplete() = default;

void Autocomplete::Start(uint64_t id, const std::string& text) {
  impl_->id = text.empty() ? 0 : id;
  if (text.empty()) {
    impl_->controller->Stop(AutocompleteStopReason::kClobbered);
    return;
  }
  impl_->controller->Start(AutocompleteInput(
      base::UTF8ToUTF16(text), metrics::OmniboxEventProto::OTHER,
      ChromeAutocompleteSchemeClassifier(impl_->profile)));
}

}  // namespace baa
