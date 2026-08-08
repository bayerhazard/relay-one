import { describe, it, expect } from "vitest";
import { get } from "svelte/store";
import { accounts } from "$lib/stores/accounts";
import type { AccountInfo } from "$lib/stores/accounts";

function makeAccount(id: number, overrides: Partial<AccountInfo> = {}): AccountInfo {
  return {
    id,
    name: `Account ${id}`,
    imap_host: "imap.test.com",
    imap_port: 993,
    smtp_host: "smtp.test.com",
    smtp_port: 465,
    username: `user${id}@test.com`,
    smtp_username: `user${id}@test.com`,
    connected: false,
    sender_name: `User ${id}`,
    sender_email: `user${id}@test.com`,
    ...overrides,
  };
}

describe("accounts store", () => {
  it("starts with empty state", () => {
    const state = get(accounts);
    expect(state.accounts).toEqual([]);
    expect(state.selectedId).toBeNull();
    expect(state.loading).toBe(false);
  });

  it("sets accounts", () => {
    const testAccounts: AccountInfo[] = [
      {
        id: 1,
        name: "Work",
        imap_host: "imap.example.com",
        imap_port: 993,
        smtp_host: "smtp.example.com",
        smtp_port: 465,
        username: "user@example.com",
        smtp_username: "user@example.com",
        connected: true,
        sender_name: "Test User",
        sender_email: "user@example.com",
      },
    ];
    accounts.setAccounts(testAccounts);
    expect(get(accounts).accounts).toEqual(testAccounts);
    expect(get(accounts).loading).toBe(false);
  });

  it("selects account by id", () => {
    accounts.selectAccount(42);
    expect(get(accounts).selectedId).toBe(42);
  });

  it("sets loading state", () => {
    accounts.setLoading(true);
    expect(get(accounts).loading).toBe(true);

    accounts.setLoading(false);
    expect(get(accounts).loading).toBe(false);
  });

  describe("CRUD operations", () => {
    it("createAccount: adds a new account to the store", () => {
      accounts.setAccounts([makeAccount(1)]);

      const current = get(accounts).accounts;
      accounts.setAccounts([...current, makeAccount(2, { name: "Personal" })]);

      const state = get(accounts);
      expect(state.accounts).toHaveLength(2);
      expect(state.accounts[0].id).toBe(1);
      expect(state.accounts[1].id).toBe(2);
      expect(state.accounts[1].name).toBe("Personal");
    });

    it("updateAccount: updates an existing account's fields", () => {
      accounts.setAccounts([
        makeAccount(1, { name: "Work", connected: false }),
        makeAccount(2, { name: "Personal" }),
      ]);

      const current = get(accounts).accounts;
      accounts.setAccounts(
        current.map((a) =>
          a.id === 1 ? { ...a, name: "Work Updated", connected: true } : a,
        ),
      );

      const state = get(accounts);
      expect(state.accounts).toHaveLength(2);
      const updated = state.accounts.find((a) => a.id === 1);
      expect(updated?.name).toBe("Work Updated");
      expect(updated?.connected).toBe(true);
      const unchanged = state.accounts.find((a) => a.id === 2);
      expect(unchanged?.name).toBe("Personal");
    });

    it("deleteAccount: removes an account from the store", () => {
      accounts.setAccounts([makeAccount(1), makeAccount(2), makeAccount(3)]);

      const current = get(accounts).accounts;
      accounts.setAccounts(current.filter((a) => a.id !== 2));

      const state = get(accounts);
      expect(state.accounts).toHaveLength(2);
      expect(state.accounts.find((a) => a.id === 2)).toBeUndefined();
      expect(state.accounts[0].id).toBe(1);
      expect(state.accounts[1].id).toBe(3);
    });
  });

  describe("loading state transitions during CRUD", () => {
    it("setAccounts clears loading after setting accounts", () => {
      accounts.setLoading(true);
      expect(get(accounts).loading).toBe(true);

      accounts.setAccounts([makeAccount(1)]);
      expect(get(accounts).loading).toBe(false);
    });

    it("tracks loading state across a full create-then-fetch cycle", () => {
      expect(get(accounts).loading).toBe(false);

      // Simulate: begin creating account → loading
      accounts.setLoading(true);
      expect(get(accounts).loading).toBe(true);

      // Simulate: add account, response received → loading cleared
      accounts.setAccounts([makeAccount(1)]);
      expect(get(accounts).loading).toBe(false);

      // Second create
      accounts.setLoading(true);
      const current = get(accounts).accounts;
      accounts.setAccounts([...current, makeAccount(2)]);
      expect(get(accounts).loading).toBe(false);
    });
  });

  describe("edge cases", () => {
    it("updateAccount: updating a non-existent account leaves the store unchanged", () => {
      accounts.setAccounts([makeAccount(1), makeAccount(2)]);

      const current = get(accounts).accounts;
      accounts.setAccounts(
        current.map((a) =>
          a.id === 99 ? { ...a, name: "Ghost" } : a,
        ),
      );

      const state = get(accounts);
      expect(state.accounts).toHaveLength(2);
      expect(state.accounts.find((a) => a.id === 99)).toBeUndefined();
      expect(state.accounts[0].name).toBe("Account 1");
    });

    it("deleteAccount: deleting a non-existent account leaves the store unchanged", () => {
      accounts.setAccounts([makeAccount(1), makeAccount(2)]);

      const current = get(accounts).accounts;
      accounts.setAccounts(current.filter((a) => a.id !== 99));

      expect(get(accounts).accounts).toHaveLength(2);
    });

    it("createAccount: adding an account with a duplicate id (array append)", () => {
      accounts.setAccounts([makeAccount(1, { name: "Original" })]);

      // Simulate creating a new entry with the same id — the store is an array
      // so duplicates are appended, not merged
      const current = get(accounts).accounts;
      accounts.setAccounts([...current, makeAccount(1, { name: "Duplicate" })]);

      const state = get(accounts);
      expect(state.accounts).toHaveLength(2);
      expect(state.accounts[0].name).toBe("Original");
      expect(state.accounts[1].name).toBe("Duplicate");
    });

    it("createAccount: can add the first account from empty state", () => {
      // setAccounts replaces the full array regardless of previous state
      accounts.setAccounts([makeAccount(1)]);

      const state = get(accounts);
      expect(state.accounts).toHaveLength(1);
      expect(state.accounts[0].id).toBe(1);
      expect(state.accounts[0].name).toBe("Account 1");
    });

    it("deleteAccount: deleting the last account leaves an empty array", () => {
      accounts.setAccounts([makeAccount(1)]);

      const current = get(accounts).accounts;
      accounts.setAccounts(current.filter((a) => a.id !== 1));

      expect(get(accounts).accounts).toEqual([]);
    });

    it("updateAccount: can update multiple fields at once", () => {
      accounts.setAccounts([makeAccount(1)]);

      const current = get(accounts).accounts;
      accounts.setAccounts(
        current.map((a) =>
          a.id === 1
            ? {
                ...a,
                name: "Renamed",
                imap_host: "new.imap.com",
                smtp_host: "new.smtp.com",
                connected: true,
                sender_name: "New Name",
              }
            : a,
        ),
      );

      const updated = get(accounts).accounts[0];
      expect(updated.name).toBe("Renamed");
      expect(updated.imap_host).toBe("new.imap.com");
      expect(updated.smtp_host).toBe("new.smtp.com");
      expect(updated.connected).toBe(true);
      expect(updated.sender_name).toBe("New Name");
      // untouched fields remain
      expect(updated.imap_port).toBe(993);
      expect(updated.smtp_port).toBe(465);
    });
  });
});
