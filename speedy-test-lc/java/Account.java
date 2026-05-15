public class Account {
    private final String id;
    private final String owner;
    private double balance;

    public Account(String id, String owner, double initialBalance) {
        this.id      = id;
        this.owner   = owner;
        this.balance = initialBalance;
    }

    public synchronized void deposit(double amount) {
        if (amount <= 0) throw new IllegalArgumentException("Deposit must be positive");
        balance += amount;
    }

    public synchronized void withdraw(double amount) {
        if (amount <= 0) throw new IllegalArgumentException("Withdrawal must be positive");
        if (amount > balance) throw new IllegalStateException("Insufficient funds");
        balance -= amount;
    }

    public synchronized double getBalance() { return balance; }
    public String getId()    { return id; }
    public String getOwner() { return owner; }

    @Override
    public String toString() {
        return String.format("Account{id=%s, owner=%s, balance=%.2f}", id, owner, balance);
    }
}
