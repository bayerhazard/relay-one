import { writable } from "svelte/store";

export interface AccountInfo {
  id: number;
  name: string;
  imap_host: string;
  imap_port: number;
  smtp_host: string;
  smtp_port: number;
  username: string;
  smtp_username: string;
  connected: boolean;
  sender_name: string;
  sender_email: string;
  sync_mode?: string;
  trash_retention_days?: number;
}

export interface AccountGroup {
  account: AccountInfo;
  collapsed: boolean;
}

interface AccountsState {
  accounts: AccountInfo[];
  groups: AccountGroup[];
  selectedId: number | null;
  loading: boolean;
}

function loadCollapsedState(): Set<number> {
  try {
    const raw = localStorage.getItem("relay_collapsed_accounts");
    if (raw) {
      return new Set(JSON.parse(raw) as number[]);
    }
  } catch { /* ignore */ }
  return new Set();
}

function createAccountsStore() {
  const { subscribe, update } = writable<AccountsState>({
    accounts: [],
    groups: [],
    selectedId: null,
    loading: false,
  });

  return {
    subscribe,
    setAccounts: (accounts: AccountInfo[]) => {
      const collapsed = loadCollapsedState();
      const groups: AccountGroup[] = accounts.map((acct, i) => ({
        account: acct,
        collapsed: i > 0 || collapsed.has(acct.id),
      }));
      update((s) => ({
        ...s,
        accounts,
        groups,
        selectedId: s.selectedId || (accounts[0]?.id ?? null),
        loading: false,
      }));
    },
    selectAccount: (id: number) => update((s) => ({ ...s, selectedId: id })),
    setLoading: (loading: boolean) => update((s) => ({ ...s, loading })),
    toggleCollapse: (accountId: number) => {
      update((s) => {
        const groups = s.groups.map((g) =>
          g.account.id === accountId ? { ...g, collapsed: !g.collapsed } : g
        );
        // Persist collapsed state
        try {
          const raw = localStorage.getItem("relay_collapsed_accounts");
          let ids: number[] = raw ? JSON.parse(raw) : [];
          const toggled = groups.find((g) => g.account.id === accountId);
          if (toggled?.collapsed && !ids.includes(accountId)) {
            ids.push(accountId);
          } else if (!toggled?.collapsed) {
            ids = ids.filter((id: number) => id !== accountId);
          }
          localStorage.setItem("relay_collapsed_accounts", JSON.stringify(ids));
        } catch { /* ignore */ }
        return { ...s, groups };
      });
    },
  };
}

export const accounts = createAccountsStore();
