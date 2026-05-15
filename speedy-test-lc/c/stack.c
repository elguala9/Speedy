#include "stack.h"
#include <stdlib.h>
#include <assert.h>

Stack *stack_create(void (*free_fn)(void *)) {
    Stack *s = malloc(sizeof(Stack));
    if (!s) return NULL;
    s->top     = NULL;
    s->size    = 0;
    s->free_fn = free_fn;
    return s;
}

void stack_destroy(Stack *s) {
    if (!s) return;
    while (!stack_is_empty(s)) {
        void *data = stack_pop(s);
        if (s->free_fn) s->free_fn(data);
    }
    free(s);
}

bool stack_push(Stack *s, void *data) {
    assert(s);
    StackNode *node = malloc(sizeof(StackNode));
    if (!node) return false;
    node->data = data;
    node->next = s->top;
    s->top     = node;
    s->size++;
    return true;
}

void *stack_pop(Stack *s) {
    assert(s);
    if (stack_is_empty(s)) return NULL;
    StackNode *node = s->top;
    void      *data = node->data;
    s->top          = node->next;
    s->size--;
    free(node);
    return data;
}

void *stack_peek(const Stack *s) {
    assert(s);
    return stack_is_empty(s) ? NULL : s->top->data;
}

bool stack_is_empty(const Stack *s) {
    assert(s);
    return s->top == NULL;
}

size_t stack_size(const Stack *s) {
    assert(s);
    return s->size;
}
