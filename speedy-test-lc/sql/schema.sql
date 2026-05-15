-- E-commerce schema: products, orders, order_items, users, reviews.

CREATE TABLE users (
    id            BIGSERIAL PRIMARY KEY,
    email         TEXT        NOT NULL UNIQUE,
    display_name  TEXT        NOT NULL,
    password_hash TEXT        NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE categories (
    id   SERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    slug TEXT NOT NULL UNIQUE
);

CREATE TABLE products (
    id          BIGSERIAL PRIMARY KEY,
    category_id INT         REFERENCES categories(id) ON DELETE SET NULL,
    name        TEXT        NOT NULL,
    slug        TEXT        NOT NULL UNIQUE,
    description TEXT,
    price_cents INT         NOT NULL CHECK (price_cents >= 0),
    stock       INT         NOT NULL DEFAULT 0 CHECK (stock >= 0),
    active      BOOLEAN     NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE orders (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT      NOT NULL REFERENCES users(id),
    status      TEXT        NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending','paid','shipped','delivered','cancelled')),
    total_cents INT         NOT NULL CHECK (total_cents >= 0),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE order_items (
    id          BIGSERIAL PRIMARY KEY,
    order_id    BIGINT  NOT NULL REFERENCES orders(id)   ON DELETE CASCADE,
    product_id  BIGINT  NOT NULL REFERENCES products(id),
    quantity    INT     NOT NULL CHECK (quantity > 0),
    unit_price_cents INT NOT NULL CHECK (unit_price_cents >= 0)
);

CREATE TABLE reviews (
    id         BIGSERIAL PRIMARY KEY,
    product_id BIGINT NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    user_id    BIGINT NOT NULL REFERENCES users(id),
    rating     SMALLINT NOT NULL CHECK (rating BETWEEN 1 AND 5),
    body       TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (product_id, user_id)
);

-- Indexes for common query patterns
CREATE INDEX idx_products_category ON products(category_id);
CREATE INDEX idx_orders_user       ON orders(user_id);
CREATE INDEX idx_reviews_product   ON reviews(product_id);

-- Average rating view
CREATE VIEW product_ratings AS
SELECT product_id,
       COUNT(*)::INT            AS review_count,
       ROUND(AVG(rating), 2)    AS avg_rating
FROM   reviews
GROUP  BY product_id;
