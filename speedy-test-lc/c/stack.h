#ifndef STACK_H
#define STACK_H

#include <stddef.h>
#include <stdbool.h>

typedef struct StackNode {
    void *data;
    struct StackNode *next;
} StackNode;

typedef struct {
    StackNode *top;
    size_t size;
    void (*free_fn)(void *);
} Stack;

Stack  *stack_create(void (*free_fn)(void *));
void    stack_destroy(Stack *s);
bool    stack_push(Stack *s, void *data);
void   *stack_pop(Stack *s);
void   *stack_peek(const Stack *s);
bool    stack_is_empty(const Stack *s);
size_t  stack_size(const Stack *s);

#endif /* STACK_H */
