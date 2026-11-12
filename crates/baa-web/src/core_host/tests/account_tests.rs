use super::*;
use crate::Account;

fn sign_ins(env: &Env, cx: &mut TestAppContext) -> Vec<(ProfileId, Account)> {
    let list = env.host.sign_ins();
    cx.run_until_parked();
    list.read_with(cx, |list, _| list.0.clone())
}

#[gpui::test]
fn a_spaces_sign_in_reaches_the_ui(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.page(3, "https://a.test", cx);
    let handle = env.host.profile_handle(ProfileId(3));
    (env.callbacks.signed_in)(env.ctx(), handle, ffi::ACCOUNT_MICROSOFT);
    assert_eq!(sign_ins(&env, cx), [(ProfileId(3), Account::Microsoft)]);
}

#[gpui::test]
fn an_incognito_sign_in_never_reaches_the_ui(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let private = env.host.ephemeral_profile();
    env.page(private.0, "https://a.test", cx);
    let handle = env.host.profile_handle(private);
    // The engine never watches an incognito profile; this side drops it anyway.
    (env.callbacks.signed_in)(env.ctx(), handle, ffi::ACCOUNT_GOOGLE);
    // Nor a profile baa does not know, nor an account it has no name for.
    (env.callbacks.signed_in)(env.ctx(), fresh(), ffi::ACCOUNT_GOOGLE);
    env.page(1, "https://b.test", cx);
    (env.callbacks.signed_in)(env.ctx(), env.host.profile_handle(ProfileId(1)), 1 << 9);
    assert_eq!(sign_ins(&env, cx), []);
}

#[gpui::test]
fn a_closed_profiles_handle_is_forgotten(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.page(1, "https://a.test", cx);
    let handle = env.host.profile_handle(ProfileId(1));
    env.host.delete_profile(ProfileId(1));
    (env.callbacks.signed_in)(env.ctx(), handle, ffi::ACCOUNT_GOOGLE);
    assert_eq!(sign_ins(&env, cx), []);
}

#[gpui::test]
fn sharing_names_the_accounts_and_the_winning_profile(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.page(2, "https://a.test", cx);
    let handle = env.host.profile_handle(ProfileId(2));
    env.host.share_accounts(&[Account::Google, Account::GitHub], Some(ProfileId(2)));
    // A Space that is not open does not win (and is not opened for it); incognito never does.
    env.host.share_accounts(&[Account::Microsoft], Some(ProfileId(7)));
    let private = env.host.ephemeral_profile();
    env.page(private.0, "https://p.test", cx);
    env.host.share_accounts(&[], Some(private));
    let google_github = ffi::ACCOUNT_GOOGLE | ffi::ACCOUNT_GITHUB;
    assert_eq!(
        calls("accounts_share"),
        [
            format!("accounts_share {google_github} {handle:?}"),
            format!("accounts_share {} {:?}", ffi::ACCOUNT_MICROSOFT, std::ptr::null_mut::<Profile>()),
            format!("accounts_share 0 {:?}", std::ptr::null_mut::<Profile>()),
        ]
    );
    assert!(calls("profile_open space-7").is_empty());
}
