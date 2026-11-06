// C ABI between BaaCore and the Rust shell. Every call runs on the main thread.
#ifndef BAA_BAA_H_
#define BAA_BAA_H_

#ifdef __cplusplus
extern "C" {
#endif

// Bumped when the ABI changes, so the Rust side can refuse a mismatched framework.
int baa_abi_version(void);

#ifdef __cplusplus
}
#endif

#endif  // BAA_BAA_H_
