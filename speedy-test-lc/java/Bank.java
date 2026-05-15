import java.util.*;
import java.util.concurrent.ConcurrentHashMap;

public class Bank {
    private final Map<String, Account> accounts = new ConcurrentHashMap<>();

    public Account openAccount(String owner, double initialDeposit) {
        if (initialDeposit < 0) throw new IllegalArgumentException("Initial deposit cannot be negative");
        Account account = new Account(UUID.randomUUID().toString(), owner, initialDeposit);
        accounts.put(account.getId(), account);
        return account;
    }

    public void transfer(String fromId, String toId, double amount) {
        Account from = getAccount(fromId);
        Account to   = getAccount(toId);
        synchronized (this) {
            from.withdraw(amount);
            to.deposit(amount);
        }
    }

    public Account getAccount(String id) {
        Account a = accounts.get(id);
        if (a == null) throw new NoSuchElementException("Account not found: " + id);
        return a;
    }

    public List<Account> listAccounts() {
        return Collections.unmodifiableList(new ArrayList<>(accounts.values()));
    }
}
