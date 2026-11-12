// Accounts shared between Spaces: baa_accounts_share and the signed_in
// callback. Each regular profile's cookies are watched; a shared account's
// changes are copied into every other one (Chrome OS's Floating SSO does the
// same between devices: chrome/browser/ash/floating_sso).

#include <array>
#include <map>
#include <memory>
#include <set>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/barrier_callback.h"
#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/no_destructor.h"
#include "base/time/time.h"
#include "chrome/browser/profiles/profile.h"
#include "content/public/browser/storage_partition.h"
#include "mojo/public/cpp/bindings/receiver.h"
#include "net/cookies/canonical_cookie.h"
#include "net/cookies/cookie_access_result.h"
#include "net/cookies/cookie_change_dispatcher.h"
#include "net/cookies/cookie_options.h"
#include "net/cookies/cookie_util.h"
#include "services/network/public/mojom/cookie_manager.mojom.h"

namespace {

// Unused slots are "".
struct Account {
  uint32_t bit;
  std::array<std::string_view, 2> domains;  // and their subdomains
  std::array<std::string_view, 3> markers;  // a cookie only a sign-in sets
};

constexpr Account kAccounts[] = {
    {BAA_ACCOUNT_GOOGLE, {"google.com"}, {"SID"}},
    {BAA_ACCOUNT_MICROSOFT,
     {"microsoftonline.com", "live.com"},
     {"ESTSAUTH", "ESTSAUTHPERSISTENT", "MSPAuth"}},
    {BAA_ACCOUNT_GITHUB, {"github.com"}, {"user_session"}},
};

const Account* AccountOf(const net::CanonicalCookie& cookie) {
  // A site's own script or a third-party embed has no business in a sign-in.
  if (cookie.IsPartitioned() ||
      cookie.SourceType() != net::CookieSourceType::kHTTP) {
    return nullptr;
  }
  std::string host = cookie.DomainWithoutDot();
  for (const Account& account : kAccounts) {
    for (std::string_view domain : account.domains) {
      if (domain.empty()) {
        continue;
      }
      if (host == domain || (host.size() > domain.size() &&
                             host.ends_with(domain) &&
                             host[host.size() - domain.size() - 1] == '.')) {
        return &account;
      }
    }
  }
  return nullptr;
}

std::string KeyOf(const net::CanonicalCookie& cookie) {
  return cookie.Name() + '\n' + cookie.Domain() + '\n' + cookie.Path();
}

// What a deletion leaves in Watch::echoes; no cookie value has a NUL.
constexpr std::string_view kGone("\0gone", 5);

uint32_t g_shared = 0;

class Watch;
std::map<BaaProfile*, std::unique_ptr<Watch>>& Watches();

// One regular profile's cookies.
class Watch : public network::mojom::CookieChangeListener {
 public:
  Watch(BaaProfile* owner, Profile* profile) : owner_(owner), profile_(profile) { Bind(); }

  network::mojom::CookieManager* Manager() {
    return profile_->GetDefaultStoragePartition()->GetCookieManagerForBrowserProcess();
  }

  void Write(const net::CanonicalCookie& cookie) {
    std::string key = KeyOf(cookie);
    echoes_[key] = cookie.Value();
    net::CookieOptions options;
    options.set_include_httponly();
    options.set_same_site_cookie_context(
        net::CookieOptions::SameSiteCookieContext::MakeInclusive());
    Manager()->SetCanonicalCookie(
        cookie, net::cookie_util::SimulatedCookieSource(cookie, "https"), options,
        base::BindOnce(
            [](base::WeakPtr<Watch> self, std::string key, std::string value,
               net::CookieAccessResult result) {
              if (self && !result.status.IsInclude()) {
                self->Forget(key, value);
              }
            },
            weak_.GetWeakPtr(), key, cookie.Value()));
  }

  void Delete(const net::CanonicalCookie& cookie) {
    std::string key = KeyOf(cookie);
    echoes_[key] = std::string(kGone);
    Manager()->DeleteCanonicalCookie(
        cookie, base::BindOnce(
                    [](base::WeakPtr<Watch> self, std::string key, bool deleted) {
                      if (self && !deleted) {
                        self->Forget(key, std::string(kGone));
                      }
                    },
                    weak_.GetWeakPtr(), key));
  }

  void OnCookieChange(const net::CookieChangeInfo& change) override {
    const net::CanonicalCookie& cookie = change.cookie;
    const Account* account = AccountOf(cookie);
    if (!account) {
      return;
    }
    std::string key = KeyOf(cookie);
    // An INSERTED right after an OVERWRITE replaced a cookie that was there: a
    // site refreshing a sign-in, not a new one.
    bool replacing = replaced_.erase(key) > 0;
    if (change.cause == net::CookieChangeCause::OVERWRITE) {
      replaced_.insert(key);
    }
    bool deletion = net::CookieChangeCauseIsDeletion(change.cause);
    // Our own Write or Delete coming back: nothing a site did.
    auto echo = echoes_.find(key);
    if (echo != echoes_.end() && echo->second == (deletion ? std::string(kGone) : cookie.Value())) {
      echoes_.erase(echo);
      return;
    }
    if (change.cause == net::CookieChangeCause::INSERTED && !replacing &&
        !(reported_ & account->bit)) {
      for (std::string_view marker : account->markers) {
        if (!marker.empty() && cookie.Name() == marker) {
          reported_ |= account->bit;
          baa::Callbacks().signed_in(baa::CallbackContext(), owner_, account->bit);
          break;
        }
      }
    }
    if (!(g_shared & account->bit)) {
      return;
    }
    switch (change.cause) {
      case net::CookieChangeCause::INSERTED:
      case net::CookieChangeCause::INSERTED_NO_VALUE_CHANGE_OVERWRITE:
        for (auto& [other, watch] : Watches()) {
          if (watch.get() != this) {
            watch->Write(cookie);
          }
        }
        break;
      // A sign-out: the site deleted it, or the user cleared it.
      case net::CookieChangeCause::EXPLICIT:
      case net::CookieChangeCause::UNKNOWN_DELETION:
      case net::CookieChangeCause::EXPIRED_OVERWRITE:
        for (auto& [other, watch] : Watches()) {
          if (watch.get() != this) {
            watch->Delete(cookie);
          }
        }
        break;
      // OVERWRITE is followed by the INSERTED that replaced it; expiry and
      // eviction happen in each profile on their own; no change is no change.
      default:
        break;
    }
  }

  base::WeakPtr<Watch> GetWeakPtr() { return weak_.GetWeakPtr(); }

 private:
  void Bind() {
    receiver_.reset();
    if (baa_is_quitting()) {
      return;  // the network service is going away with everything else
    }
    Manager()->AddGlobalChangeListener(receiver_.BindNewPipeAndPassRemote());
    // The network service restarted (it crashes): listen to its new one.
    receiver_.set_disconnect_handler(
        base::BindOnce(&Watch::Bind, base::Unretained(this)));
  }

  // ponytail: an echo whose write was refused, or that changed nothing (no
  // event), stays until the next event for its cookie, which it then hides if
  // the value matches. That change is then not copied; harmless, as the value
  // it would copy is that same one.
  void Forget(const std::string& key, const std::string& value) {
    auto it = echoes_.find(key);
    if (it != echoes_.end() && it->second == value) {
      echoes_.erase(it);
    }
  }

  const raw_ptr<BaaProfile> owner_;
  const raw_ptr<Profile> profile_;
  uint32_t reported_ = 0;
  // Cookies we just wrote (their value) or deleted (kGone), by KeyOf.
  std::map<std::string, std::string> echoes_;
  std::set<std::string> replaced_;
  mojo::Receiver<network::mojom::CookieChangeListener> receiver_{this};
  base::WeakPtrFactory<Watch> weak_{this};
};

std::map<BaaProfile*, std::unique_ptr<Watch>>& Watches() {
  static base::NoDestructor<std::map<BaaProfile*, std::unique_ptr<Watch>>> watches;
  return *watches;
}

using List = std::pair<base::WeakPtr<Watch>, net::CookieList>;

// Gives every watched profile the winning copy of each `accounts` cookie:
// `preferred`'s where it has one, else the most recently updated.
// ponytail: a cookie one profile deleted while the others were closed comes
// back from them; Google, Microsoft and GitHub then sign that profile in again.
void Spread(uint32_t accounts, base::WeakPtr<Watch> preferred, std::vector<List> lists) {
  struct Winner {
    net::CanonicalCookie cookie;
    bool preferred;
  };
  std::map<std::string, Winner> winners;
  base::Time now = base::Time::Now();
  for (auto& [owner, list] : lists) {
    if (!owner) {
      continue;  // closed meanwhile
    }
    for (net::CanonicalCookie& cookie : list) {
      const Account* account = AccountOf(cookie);
      if (!account || !(accounts & account->bit) || cookie.IsExpired(now)) {
        continue;
      }
      bool ours = owner.get() == preferred.get();
      auto [it, added] = winners.try_emplace(KeyOf(cookie), Winner{cookie, ours});
      if (!added && !it->second.preferred &&
          (ours || cookie.LastUpdateDate() > it->second.cookie.LastUpdateDate())) {
        it->second = Winner{cookie, ours};
      }
    }
  }
  for (auto& [owner, list] : lists) {
    if (!owner) {
      continue;
    }
    std::map<std::string, const net::CanonicalCookie*> have;
    for (const net::CanonicalCookie& cookie : list) {
      have[KeyOf(cookie)] = &cookie;
    }
    for (auto& [key, winner] : winners) {
      auto it = have.find(key);
      if (it == have.end() || it->second->Value() != winner.cookie.Value() ||
          it->second->ExpiryDate() != winner.cookie.ExpiryDate()) {
        owner->Write(winner.cookie);
      }
    }
  }
}

void Merge(base::WeakPtr<Watch> preferred) {
  if (!g_shared || Watches().size() < 2) {
    return;
  }
  auto done = base::BarrierCallback<List>(
      Watches().size(), base::BindOnce(&Spread, g_shared, preferred));
  for (auto& [owner, watch] : Watches()) {
    watch->Manager()->GetAllCookies(base::BindOnce(
        [](base::WeakPtr<Watch> watch, base::RepeatingCallback<void(List)> done,
           const net::CookieList& list) { done.Run({watch, list}); },
        watch->GetWeakPtr(), done));
  }
}

}  // namespace

namespace baa {

void WatchAccounts(BaaProfile* owner, Profile* profile) {
  if (profile->IsOffTheRecord()) {
    return;  // incognito: nothing leaves it, nothing comes in
  }
  Watches()[owner] = std::make_unique<Watch>(owner, profile);
  Merge(nullptr);
}

void UnwatchAccounts(BaaProfile* owner) {
  Watches().erase(owner);
}

}  // namespace baa

void baa_accounts_share(uint32_t accounts, BaaProfile* from) {
  g_shared = accounts;
  auto watch = Watches().find(from);
  Merge(watch != Watches().end() ? watch->second->GetWeakPtr() : nullptr);
}
