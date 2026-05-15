package speedytest

import scala.collection.mutable

/** Immutable transformation pipeline over a lazy stream of records. */
final class Pipeline[A] private (private val stages: List[A => Option[A]]) {

  def filter(p: A => Boolean): Pipeline[A] =
    new Pipeline(stages :+ (a => if (p(a)) Some(a) else None))

  def map[B >: A](f: A => B): Pipeline[B] =
    new Pipeline[B](stages.asInstanceOf[List[B => Option[B]]] :+ (b => Some(f(b.asInstanceOf[A]).asInstanceOf[B])))

  def run(source: Iterable[A]): List[A] =
    source.foldLeft(mutable.ListBuffer.empty[A]) { (acc, item) =>
      stages.foldLeft(Option(item)) { (opt, stage) => opt.flatMap(stage) }
        .foreach(acc += _)
      acc
    }.toList
}

object Pipeline {
  def of[A]: Pipeline[A] = new Pipeline(Nil)
}

// Example: word-frequency counter
object WordCount extends App {
  val lines = List(
    "the quick brown fox",
    "the fox jumped over the lazy dog",
    "dogs and foxes",
  )

  val words = Pipeline
    .of[String]
    .filter(_.nonEmpty)
    .run(lines.flatMap(_.split("\\s+")))

  val freq = words.groupMapReduce(identity)(_ => 1)(_ + _)
  freq.toSeq.sortBy(-_._2).take(5).foreach { case (w, n) => println(s"$w: $n") }
}
