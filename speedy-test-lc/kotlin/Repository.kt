package speedytest

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.map

data class User(
    val id: Long,
    val name: String,
    val email: String,
    val role: Role,
)

enum class Role { ADMIN, EDITOR, VIEWER }

interface UserRepository {
    fun findAll(): Flow<List<User>>
    fun findById(id: Long): User?
    suspend fun save(user: User): User
    suspend fun delete(id: Long): Boolean
}

class InMemoryUserRepository : UserRepository {
    private val store = MutableStateFlow<Map<Long, User>>(emptyMap())
    private var nextId = 1L

    override fun findAll(): Flow<List<User>> =
        store.map { it.values.sortedBy(User::id) }

    override fun findById(id: Long): User? = store.value[id]

    override suspend fun save(user: User): User {
        val saved = if (user.id == 0L) user.copy(id = nextId++) else user
        store.value = store.value + (saved.id to saved)
        return saved
    }

    override suspend fun delete(id: Long): Boolean {
        val existed = store.value.containsKey(id)
        store.value = store.value - id
        return existed
    }
}
