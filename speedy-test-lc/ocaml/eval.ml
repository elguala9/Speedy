(** Simple arithmetic expression evaluator. *)

type expr =
  | Num   of float
  | Var   of string
  | Add   of expr * expr
  | Sub   of expr * expr
  | Mul   of expr * expr
  | Div   of expr * expr
  | Let   of string * expr * expr

type env = (string * float) list

exception Unbound_variable of string
exception Division_by_zero

let rec eval env = function
  | Num n          -> n
  | Var x          ->
    (match List.assoc_opt x env with
     | Some v -> v
     | None   -> raise (Unbound_variable x))
  | Add (l, r)     -> eval env l +. eval env r
  | Sub (l, r)     -> eval env l -. eval env r
  | Mul (l, r)     -> eval env l *. eval env r
  | Div (l, r)     ->
    let d = eval env r in
    if d = 0.0 then raise Division_by_zero
    else eval env l /. d
  | Let (x, e, body) ->
    let v = eval env e in
    eval ((x, v) :: env) body

let () =
  (* let x = 4.0 in let y = x * 2.5 in x + y *)
  let program =
    Let ("x", Num 4.0,
      Let ("y", Mul (Var "x", Num 2.5),
        Add (Var "x", Var "y"))) in
  let result = eval [] program in
  Printf.printf "Result: %g\n" result
