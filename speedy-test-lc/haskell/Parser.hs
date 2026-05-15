module Parser where

import Control.Applicative (Alternative (..))
import Data.Char (isDigit, isAlpha, isSpace)

-- | A simple parser-combinator library.
newtype Parser a = Parser { runParser :: String -> Maybe (a, String) }

instance Functor Parser where
  fmap f (Parser p) = Parser $ \input -> do
    (a, rest) <- p input
    pure (f a, rest)

instance Applicative Parser where
  pure a = Parser $ \input -> Just (a, input)
  Parser pf <*> Parser pa = Parser $ \input -> do
    (f, rest1) <- pf input
    (a, rest2) <- pa rest1
    pure (f a, rest2)

instance Alternative Parser where
  empty = Parser $ const Nothing
  Parser p <|> Parser q = Parser $ \input -> p input <|> q input

instance Monad Parser where
  return = pure
  Parser p >>= f = Parser $ \input -> do
    (a, rest) <- p input
    runParser (f a) rest

satisfy :: (Char -> Bool) -> Parser Char
satisfy pred = Parser $ \case
  (c:cs) | pred c -> Just (c, cs)
  _               -> Nothing

char :: Char -> Parser Char
char c = satisfy (== c)

digit :: Parser Char
digit = satisfy isDigit

letter :: Parser Char
letter = satisfy isAlpha

spaces :: Parser String
spaces = many (satisfy isSpace)

natural :: Parser Int
natural = read <$> some digit

identifier :: Parser String
identifier = (:) <$> letter <*> many (letter <|> digit <|> char '_')

between :: Parser a -> Parser b -> Parser c -> Parser c
between open close p = open *> p <* close

sepBy :: Parser a -> Parser b -> Parser [a]
sepBy p sep = ((:) <$> p <*> many (sep *> p)) <|> pure []

csv :: Parser [[String]]
csv = sepBy row (char '\n')
  where row  = sepBy cell (char ',')
        cell = many (satisfy (/= ','))
