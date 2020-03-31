use pest::Parser;
use super::ast::*;
use super::name::Name;
use std::path::Path;
use std::io::{Read};
use super::pp::PP;
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use pest::prec_climber::{Operator, PrecClimber, Assoc};
use super::make::Stage;

#[derive(Parser)]
#[grammar = "zz.pest"]
pub struct ZZParser;

pub static ERRORS_AS_JSON : AtomicBool = AtomicBool::new(false);



pub fn parse(n: &Path, features: &HashMap<String, bool>, stage: &Stage) -> Module
{
    match p(&n, features, stage){
        Err(e) => {
            let e = e.with_path(&n.to_string_lossy().to_string());
            if ERRORS_AS_JSON.load(Ordering::SeqCst) {


                let mut j = JsonError::default();
                j.message       = format!("syntax error:\n{}", e);
                j.level         = "error".to_string();
                j.file_name     = n.to_string_lossy().to_string();

                match e.line_col {
                    pest::error::LineColLocation::Span((l1,c1),(l2,c2)) => {
                        j.line_start    = l1;
                        j.column_start  = c1;
                        j.line_end      = l2;
                        j.column_end    = c2;
                    },
                    pest::error::LineColLocation::Pos((l1,c1)) => {
                        j.line_start    = l1;
                        j.column_start  = c1;
                        j.line_end      = l1;
                        j.column_end    = c1;
                    }
                };
                println!("{}", serde_json::to_string(&j).unwrap());
            } else {
                error!("syntax error\n{}", e);
            }
            std::process::exit(9);
        }
        Ok(md) => {
            md
        }
    }
}

fn p(n: &Path, features: &HashMap<String, bool> , stage: &Stage) -> Result<Module, pest::error::Error<Rule>> {

    let mut module = Module::default();
    module.source = n.to_path_buf();
    module.sources.insert(n.canonicalize().unwrap());
    module.name.push(n.file_stem().expect(&format!("stem {:?}", n)).to_string_lossy().into());

    let n = &n.to_string_lossy().to_string();
    let (file_str,_) = read_source(n.clone());

    let mut file = ZZParser::parse(Rule::file, file_str)?;
    let mut doccomments = String::new();

    for decl in PP::new(&n, features.clone(), stage.clone(), file.next().unwrap().into_inner()) {
        match decl.as_rule() {
            Rule::doccomment => {
                let mut s = decl.as_str().to_string();
                s.remove(0);
                s.remove(0);
                doccomments.push_str(&s);
            }
            Rule::imacro => {
                let loc = Location::from_span(n.into(), &decl.as_span());
                let decl = decl.into_inner();
                let mut name = None;
                let mut args = Vec::new();
                let mut body = None;
                let mut vis = Visibility::Object;
                for part in decl {
                    match part.as_rule() {
                        Rule::key_shared => {
                            vis = Visibility::Shared;
                        }
                        Rule::exported => {
                            vis = Visibility::Export;
                        }
                        Rule::ident if name.is_none() => {
                            name = part.as_str().into();
                        }
                        Rule::macro_args => {
                            for arg in part.into_inner() {
                                args.push(arg.as_str().into());
                            }
                        }
                        Rule::block if body.is_none() => {
                            body = Some(parse_block(&n, features, stage, part));
                        },
                        e => panic!("unexpected rule {:?} in macro ", e),
                    }
                }

                module.locals.push(Local{
                    name: name.unwrap().to_string(),
                    vis,
                    loc,
                    def:  Def::Macro{
                        args,
                        body: body.unwrap(),
                    },
                    doc: std::mem::replace(&mut doccomments, String::new()),
                });

            }
            Rule::function | Rule::fntype | Rule::theory => {
                let loc = Location::from_span(n.into(), &decl.as_span());
                let mut nameloc = loc.clone();
                let declrule = decl.as_rule().clone();
                let decl = decl.into_inner();
                let mut name = String::new();
                let mut args = Vec::new();
                let mut ret  = None;
                let mut body = None;
                let mut attr = HashMap::new();
                let mut vararg = false;
                let mut callassert = Vec::new();
                let mut calleffect = Vec::new();
                let mut vis = Visibility::Object;
                let mut derives = Vec::new();

                for part in decl {
                    match part.as_rule() {
                        Rule::key_shared => {
                            vis = Visibility::Shared;
                        }
                        Rule::exported => {
                            vis = Visibility::Export;
                        }
                        Rule::ident => {
                            nameloc = Location::from_span(n.into(), &part.as_span());
                            name = part.as_str().into();
                        }
                        Rule::ret_arg => {
                            let part = part.into_inner().next().unwrap();
                            ret = Some(AnonArg{
                                typed: parse_anon_type(n, part),
                            });
                        },
                        Rule::fn_attr => {
                            let loc = Location::from_span(n.into(), &part.as_span());
                            attr.insert(part.as_str().into(), loc);
                        },
                        Rule::fn_args => {
                            for arg in part.into_inner() {

                                let argloc = Location::from_span(n.into(), &arg.as_span());

                                if arg.as_rule() == Rule::vararg {
                                    vararg = true;
                                } else {
                                    let TypedName{typed, name, tags} = parse_named_type(n, arg);

                                    args.push(NamedArg{
                                        name,
                                        typed,
                                        tags,
                                        loc: argloc,
                                    });
                                }
                            }
                        },
                        Rule::call_assert => {
                            let part = part.into_inner().next().unwrap();
                            callassert.push(parse_expr(n, part));
                        },
                        Rule::call_effect => {
                            let part = part.into_inner().next().unwrap();
                            calleffect.push(parse_expr(n, part));
                        },
                        Rule::block => {
                            body = Some(parse_block(n, features, stage, part));
                        },
                        Rule::macrocall => {
                            derives.push(parse_derive(n, part));
                        }
                        e => panic!("unexpected rule {:?} in function", e),
                    }
                }

                match declrule {
                    Rule::function => {
                        module.locals.push(Local{
                            doc: std::mem::replace(&mut doccomments, String::new()),
                            name,
                            vis,
                            loc,
                            def:Def::Function{
                                nameloc,
                                ret,
                                attr,
                                derives,
                                args,
                                body: body.unwrap(),
                                vararg,
                                callassert,
                                calleffect,
                                callattests: Vec::new(),
                            }
                        });
                    }
                    Rule::theory => {
                        module.locals.push(Local{
                            doc: std::mem::replace(&mut doccomments, String::new()),
                            name,
                            vis,
                            loc,
                            def:Def::Theory{
                                ret,
                                attr,
                                args,
                            }
                        });
                    },
                    Rule::fntype => {
                        module.locals.push(Local{
                            doc: std::mem::replace(&mut doccomments, String::new()),
                            name,
                            vis,
                            loc,
                            def:Def::Fntype{
                                nameloc,
                                ret,
                                attr,
                                args,
                                vararg,
                            }
                        });
                    },
                    _ => unreachable!()
                }
            },
            Rule::EOI => {},
            Rule::ienum => {
                let decl = decl.into_inner();

                let mut vis    = Visibility::Object;
                let mut name   = None;
                let mut names  = Vec::new();
                let mut loc    = None;

                for part in PP::new(n, features.clone(), stage.clone(), decl) {
                    match part.as_rule() {
                        Rule::key_shared => {
                            vis = Visibility::Shared;
                        }
                        Rule::exported => {
                            vis = Visibility::Export;
                        }
                        Rule::ident if name.is_none() => {
                            loc = Some(Location::from_span(n.into(), &part.as_span()));
                            name = Some(part.as_str().into());

                        }
                        Rule::enum_i => {
                            let mut part = part.into_inner();
                            let name = part.next().unwrap().as_str().to_string();
                            let mut literal = None;
                            if let Some(part) = part.next() {
                                literal = Some(match part.as_str().to_string().parse() {
                                    Err(e) => {
                                        let loc = Location::from_span(n.into(), &part.as_span());
                                        emit_error(
                                            "enums must be positive integer literals",
                                            &[(loc, format!("{}", e))]
                                        );
                                        std::process::exit(9);
                                    },
                                    Ok(v) => v,
                                });
                            }

                            names.push((name, literal));


                        }
                        e => panic!("unexpected rule {:?} in enum", e),
                    }
                };

                module.locals.push(Local{
                    doc: std::mem::replace(&mut doccomments, String::new()),
                    name: name.unwrap(),
                    vis,
                    loc: loc.unwrap(),
                    def: Def::Enum{
                        names,
                    }
                });

            },
            Rule::testcase => {
                let mut name   = None;
                let mut fields = Vec::new();
                let mut loc = Location::from_span(n.into(), &decl.as_span());

                let decl = decl.into_inner();
                for part in PP::new(n,features.clone(), stage.clone(), decl) {
                    match part.as_rule() {
                        Rule::ident => {
                            loc = Location::from_span(n.into(), &part.as_span());
                            name= Some(part.as_str().into());
                        },
                        Rule::testfield => {
                            let mut part = part.into_inner();
                            let fname   = part.next().unwrap().as_str().to_string();
                            let _op      = part.next().unwrap().as_str().to_string();
                            let expr    = parse_expr(n, part.next().unwrap());
                            fields.push((fname,expr));
                        }
                        e => panic!("unexpected rule {:?} in testcase", e),

                    }
                }
                module.locals.push(Local{
                    doc: std::mem::replace(&mut doccomments, String::new()),
                    name: name.unwrap_or(format!("anonymous_test_case_{}", loc.line)),
                    vis: Visibility::Object,
                    loc,
                    def: Def::Testcase {
                        fields,
                    }
                });

            },

            Rule::struct_d => {
                let decl = decl.into_inner();

                let mut vis    = Visibility::Object;
                let mut name   = None;
                let mut fields = Vec::new();
                let mut loc    = None;
                let mut packed = false;
                let mut tail   = Tail::None;
                let mut union  = false;

                for part in PP::new(n,features.clone(), stage.clone(), decl) {
                    match part.as_rule() {
                        Rule::tail => {
                            tail = Tail::Dynamic;
                        }
                        Rule::key_packed => {
                            packed = true;
                        }
                        Rule::key_shared => {
                            vis = Visibility::Shared;
                        }
                        Rule::key_struct => {
                            union = false;
                        }
                        Rule::key_union => {
                            union = true;
                        }
                        Rule::exported => {
                            vis = Visibility::Export;
                        }
                        Rule::ident => {
                            loc = Some(Location::from_span(n.into(), &part.as_span()));
                            name= Some(part.as_str().into());
                        }
                        Rule::struct_f => {

                            let loc = Location::from_span(n.into(), &part.as_span());


                            let mut part = part.into_inner();

                            let TypedName{typed, name, tags} = parse_named_type(n, part.next().unwrap());

                            let array = match part.next() {
                                None => Array::None,
                                Some(array) => {
                                    match array.into_inner().next() {
                                        Some(expr) => {
                                            Array::Sized(parse_expr(n, expr))
                                        },
                                        None => {
                                            Array::Unsized
                                        }
                                    }
                                }
                            };


                            fields.push(Field{
                                typed,
                                array,
                                tags,
                                name,
                                loc,
                            });
                        }
                        e => panic!("unexpected rule {:?} in struct ", e),
                    }
                };



                module.locals.push(Local{
                    doc: std::mem::replace(&mut doccomments, String::new()),
                    name: name.unwrap(),
                    vis,
                    loc: loc.unwrap(),
                    def: Def::Struct {
                        fields,
                        packed,
                        tail,
                        union,
                        impls: HashMap::new(),
                    }
                });
            }
            Rule::import => {
                let loc = Location::from_span(n.into(), &decl.as_span());
                let mut vis = Visibility::Object;
                let mut importname = None;
                let mut alias      = None;
                let mut inline     = false;
                let mut needs      = Vec::new();

                for part in decl.into_inner() {
                    match part.as_rule() {
                        Rule::importname => {
                            importname = Some(parse_importname(part));
                        },
                        Rule::exported => {
                            vis = Visibility::Export;
                        }
                        Rule::key_shared => {
                            vis = Visibility::Shared;
                        }
                        Rule::importalias => {
                            alias = Some(part.into_inner().next().unwrap().as_str().to_string());
                        }
                        Rule::key_inline => {
                            inline = true;
                        }
                        Rule::importdeps => {
                            for ident in part.into_inner() {
                                needs.push((
                                        Typed{
                                            t:      Type::Other(Name::from(ident.as_str())),
                                            ptr:    Vec::new(),
                                            loc:    Location::from_span(n.into(), &ident.as_span()),
                                            tail:   Tail::None,
                                        },
                                        Location::from_span(n.into(), &ident.as_span()),
                                ));
                            }
                        }
                        e => panic!("unexpected rule {:?} in import ", e),
                    }
                };

                let (name, local) = importname.unwrap();
                module.imports.push(Import{
                    name,
                    alias,
                    local,
                    vis,
                    loc,
                    inline,
                    needs,
                });


            },
            Rule::comment => {},
            Rule::istatic | Rule::constant => {
                let rule = decl.as_rule();
                let loc = Location::from_span(n.into(), &decl.as_span());
                let mut storage = Storage::Static;
                let mut vis     = Visibility::Object;
                let mut typed   = None;
                let mut expr    = None;
                let mut array   = Array::None;

                for part in decl.into_inner() {
                    match part.as_rule() {
                        Rule::key_thread_local => {
                            storage = Storage::ThreadLocal;
                        }
                        Rule::key_static => {
                            storage = Storage::Static;
                        }
                        Rule::key_atomic => {
                            storage = Storage::Atomic;
                        }
                        Rule::key_shared =>  {
                            if let Rule::istatic = rule {
                                let e = pest::error::Error::<Rule>::new_from_span(pest::error::ErrorVariant::CustomError {
                                    message: format!("cannot change visibility of static variable"),
                                }, part.as_span());
                                error!("{} : {}", n, e);
                                std::process::exit(9);
                            } else {
                                vis = Visibility::Shared;
                            }
                        }
                        Rule::exported => {
                            if let Rule::istatic = rule {
                                let e = pest::error::Error::<Rule>::new_from_span(pest::error::ErrorVariant::CustomError {
                                    message: format!("cannot change visibility of static variable"),
                                }, part.as_span());
                                error!("{} : {}", n, e);
                                std::process::exit(9);
                            } else {
                                vis = Visibility::Export;
                            }
                        },
                        Rule::named_type => {
                            typed = Some(parse_named_type(n, part));
                        },
                        Rule::expr if expr.is_none() => {
                            expr = Some(parse_expr(n, part));
                        }
                        Rule::array => {
                            if let Some(expr) = part.into_inner().next() {
                                array = Array::Sized(parse_expr(n, expr));
                            } else {
                                array = Array::Unsized;
                            }
                        }
                        e => panic!("unexpected rule {:?} in static", e),
                    }
                }

                let TypedName{typed, name, tags} = typed.unwrap();
                match rule {

                    Rule::constant => {
                        for (_,tag) in tags.0 {
                            emit_error("syntax error", &[(
                                       tag.iter().next().unwrap().1.clone(),
                                       "anonymous type cannot have storage tags (yet)")]);
                            std::process::exit(9);
                        }

                        module.locals.push(Local{
                            doc: std::mem::replace(&mut doccomments, String::new()),
                            name: name,
                            loc,
                            vis,
                            def: Def::Const {
                                typed,
                                expr: expr.unwrap(),
                            }
                        });
                    },
                    Rule::istatic => {
                        module.locals.push(Local{
                            doc: std::mem::replace(&mut doccomments, String::new()),
                            name: name,
                            loc,
                            vis: Visibility::Object,
                            def: Def::Static {
                                array,
                                tags,
                                storage,
                                typed,
                                expr: expr.unwrap(),
                            }
                        });
                    },
                    _ => unreachable!(),
                }
            },
            e => panic!("unexpected rule {:?} in file", e),

        }

    }

    Ok(module)
}

pub(crate) fn parse_derive(n: &str, decl: pest::iterators::Pair<'static, Rule>) -> Derive {
    match decl.as_rule() {
        Rule::macrocall=> { }
        _ => { panic!("parse_expr call called with {:?}", decl); }
    };
    let loc = Location::from_span(n.into(), &decl.as_span());

    let mut decl = decl.into_inner();
    let makro = decl.next().unwrap().as_str().to_string();

    let mut args = Vec::new();
    if let Some(callargs) = decl.next() {
        for part in callargs.into_inner() {
            args.push(Box::new(parse_expr(n, part)));
        }
    }

    Derive {
        loc,
        makro,
        args
    }
}


pub(crate) fn parse_expr(n: &str, decl: pest::iterators::Pair<'static, Rule>) -> Expression {
    match decl.as_rule() {
        Rule::expr  => { }
        Rule::expr_to_precedence_2 => {}
        _ => { panic!("parse_expr call called with {:?}", decl); }
    };


    let climber = PrecClimber::new(vec![
        //12
        Operator::new(Rule::boolor, Assoc::Left),
        //11
        Operator::new(Rule::booland, Assoc::Left),
        //10
        Operator::new(Rule::bitor, Assoc::Left),
        //9
        Operator::new(Rule::bitxor, Assoc::Left),
        //8
        Operator::new(Rule::bitand, Assoc::Left),
        //7
        Operator::new(Rule::equals, Assoc::Left) | Operator::new(Rule::nequals, Assoc::Left),
        //6
        Operator::new(Rule::lessthan, Assoc::Left) | Operator::new(Rule::morethan, Assoc::Left) |
        Operator::new(Rule::moreeq, Assoc::Left) | Operator::new(Rule::lesseq, Assoc::Left),
        //5
        Operator::new(Rule::shiftleft, Assoc::Left) | Operator::new(Rule::shiftright, Assoc::Left),
        //4
        Operator::new(Rule::add, Assoc::Left) | Operator::new(Rule::subtract, Assoc::Left),
        //3
        Operator::new(Rule::modulo, Assoc::Left) | Operator::new(Rule::divide, Assoc::Left) |
        Operator::new(Rule::multiply, Assoc::Left),
        //2
        Operator::new(Rule::preop, Assoc::Right),
        //1
        Operator::new(Rule::memberaccess,   Assoc::Left) |
            Operator::new(Rule::ptraccess,  Assoc::Left) |
            Operator::new(Rule::callstart,  Assoc::Left) |
            Operator::new(Rule::arraystart, Assoc::Left),

    ]);

    let reduce = |lhs: Expression, op: pest::iterators::Pair<'static, Rule>, rhs: Expression | {

        let loc = Location::from_span(n.into(), &op.as_span());

        if op.as_rule()  == Rule::memberaccess {
            if let Expression::Name(typed) = &rhs {
                if let Type::Other(n) = &typed.t {
                    return Expression::MemberAccess{
                        lhs: Box::new(lhs),
                        rhs: n.to_string(),
                        op:  ".".to_string(),
                        loc: loc.clone(),
                    };
                }
            }
            emit_error(format!("ICE: unexpected rhs {:?}", rhs), &[
                       (loc.clone(), "in this memberaccess ")
            ]);
            std::process::exit(9);
        } else if op.as_rule()  == Rule::ptraccess {
            if let Expression::Name(typed) = &rhs {
                if let Type::Other(n) = &typed.t {
                    return Expression::MemberAccess{
                        lhs: Box::new(lhs),
                        rhs: n.to_string(),
                        op:  "->".to_string(),
                        loc: loc.clone(),
                    };
                }
            }
            emit_error(format!("ICE: unexpected rhs {:?}", rhs), &[
                       (loc.clone(), "in this ptraccess ")
            ]);
            std::process::exit(9);
        } else if op.as_rule()  == Rule::callstart {
            if let Expression::Call{loc, args, .. }  = &rhs {
                return Expression::Call{
                    loc:            loc.clone(),
                    name:           Box::new(lhs),
                    args:           args.clone(),
                    expanded:       false,
                    emit:           EmitBehaviour::Default,
                };
            }
            emit_error(format!("ICE: unexpected rhs {:?}", rhs), &[
                       (loc.clone(), "in this call ")
            ]);
            std::process::exit(9);
        } else if op.as_rule()  == Rule::arraystart {
            return Expression::ArrayAccess {
                loc:    loc.clone(),
                lhs:    Box::new(lhs),
                rhs:    Box::new(rhs),
            };
        }

        Expression::Infix {
            loc:    loc.clone(),
            lhs:    Box::new(lhs),
            rhs:    Box::new(rhs),
            op:     match op.as_rule() {
                Rule::equals    => crate::ast::InfixOperator::Equals,
                Rule::nequals   => crate::ast::InfixOperator::Nequals,
                Rule::add       => crate::ast::InfixOperator::Add,
                Rule::subtract  => crate::ast::InfixOperator::Subtract,
                Rule::multiply  => crate::ast::InfixOperator::Multiply,
                Rule::divide    => crate::ast::InfixOperator::Divide,
                Rule::bitxor    => crate::ast::InfixOperator::Bitxor,
                Rule::booland   => crate::ast::InfixOperator::Booland,
                Rule::boolor    => crate::ast::InfixOperator::Boolor,
                Rule::moreeq    => crate::ast::InfixOperator::Moreeq,
                Rule::lesseq    => crate::ast::InfixOperator::Lesseq,
                Rule::lessthan  => crate::ast::InfixOperator::Lessthan,
                Rule::morethan  => crate::ast::InfixOperator::Morethan,
                Rule::shiftleft => crate::ast::InfixOperator::Shiftleft,
                Rule::shiftright=> crate::ast::InfixOperator::Shiftright,
                Rule::modulo    => crate::ast::InfixOperator::Modulo,
                Rule::bitand    => crate::ast::InfixOperator::Bitand,
                Rule::bitor     => crate::ast::InfixOperator::Bitor,
                _ => {
                    emit_error(format!("ICE: unexpected operator {}", op), &[
                        (loc.clone(), "in this infix")
                    ]);
                    std::process::exit(9);
                }
            },
        }
    };
    climber.climb(decl.into_inner(), |pair|parse_expr_inner(n, pair), reduce)
}


pub(crate) fn parse_expr_inner(n: &str, expr: pest::iterators::Pair<'static, Rule>) -> Expression {

    let loc = Location::from_span(n.into(), &expr.as_span());

    let asrule = expr.as_rule();
    match asrule {
        Rule::unarypre => {
            let mut expr = expr.into_inner();
            let part    = expr.next().unwrap();
            let op      = match part.as_rule() {
                Rule::boolnot   => crate::ast::PrefixOperator::Boolnot,
                Rule::bitnot    => crate::ast::PrefixOperator::Bitnot,
                Rule::increment => crate::ast::PrefixOperator::Increment,
                Rule::decrement => crate::ast::PrefixOperator::Decrement,
                _ => {
                    emit_error("ICE: unexpected operator", &[
                               (loc.clone(), "in this expr")
                    ]);
                    std::process::exit(9);
                }
            };
            let part   = expr.next().unwrap();
            let iexpr   = match part.as_rule() {
                Rule::type_name => {
                    let loc = Location::from_span(n.into(), &part.as_span());
                    let name = Name::from(part.as_str());
                    Expression::Name(Typed{
                        t:   Type::Other(name),
                        ptr: Vec::new(),
                        loc,
                        tail: Tail::None,
                    })
                },
                Rule::expr_to_precedence_2 => {
                    parse_expr(n, part)
                }
                e => panic!("unexpected rule {:?} in unary pre lhs", e),
            };
            Expression::UnaryPre{
                expr: Box::new(iexpr),
                op,
                loc,
            }
        },
        Rule::unarypost => {
            let mut expr = expr.into_inner();
            let part   = expr.next().unwrap();
            let iexpr   = match part.as_rule() {
                Rule::type_name => {
                    let loc = Location::from_span(n.into(), &part.as_span());
                    let name = Name::from(part.as_str());
                    Expression::Name(Typed{
                        t:   Type::Other(name),
                        ptr: Vec::new(),
                        loc,
                        tail: Tail::None,
                    })
                },
                Rule::expr => {
                    parse_expr(n, part)
                }
                e => panic!("unexpected rule {:?} in unary post lhs", e),
            };

            let part    = expr.next().unwrap();
            let op      = match part.as_rule() {
                Rule::increment => crate::ast::PostfixOperator::Increment,
                Rule::decrement => crate::ast::PostfixOperator::Decrement,
                _ => {
                    emit_error("ICE: unexpected operator", &[
                               (loc.clone(), "in this expr")
                    ]);
                    std::process::exit(9);
                }
            };

            Expression::UnaryPost{
                op,
                expr: Box::new(iexpr),
                loc,
            }
        },
        Rule::cast => {
            let mut expr = expr.into_inner();
            let part  = expr.next().unwrap();
            let into = parse_anon_type(n, part);
            let part  = expr.next().unwrap();
            let expr = parse_expr(n, part);
            Expression::Cast{
                loc,
                into,
                expr: Box::new(expr),
            }
        },
        Rule::type_name => {
            let name = Name::from(expr.as_str());
            Expression::Name(Typed{
                t:   Type::Other(name),
                ptr: Vec::new(),
                loc,
                tail: Tail::None,
            })
        },
        Rule::string_literal => {
            let mut val = expr.as_str().to_string();
            let v = if val.starts_with("r#") {
                val.remove(0);
                val.remove(0);
                val.remove(0);
                val.pop();
                val.pop();
                val.as_bytes().to_vec()
            } else {
                val.remove(0);
                val.pop();
                unescape(&val, &loc)
            };

            Expression::LiteralString {
                v: String::from_utf8_lossy(&v).to_string(),
                loc,
            }
        }
        Rule::char_literal => {
            let mut val = expr.as_str().to_string();
            val.remove(0);
            val.pop();
            let v = unescape(&val, &loc);

            Expression::LiteralChar {
                v: v[0],
                loc,
            }
        }
        Rule::number_literal | Rule::bool_literal=> {
            Expression::Literal {
                v: expr.as_str().to_string(),
                loc,
            }
        },
        Rule::expr => {
            parse_expr(n, expr)
        },
        Rule::deref | Rule::takeref => {
            let op = match expr.as_rule() {
                Rule::deref   => crate::ast::PrefixOperator::Deref,
                Rule::takeref => crate::ast::PrefixOperator::AddressOf,
                _ => unreachable!(),
            };

            let part = expr.into_inner().next().unwrap();
            let expr = match part.as_rule() {
                Rule::type_name => {
                    let loc = Location::from_span(n.into(), &part.as_span());
                    let name = Name::from(part.as_str());
                    Expression::Name(Typed{
                        t:   Type::Other(name),
                        ptr: Vec::new(),
                        loc,
                        tail: Tail::None,
                    })
                },
                Rule::expr_to_precedence_2 => {
                    parse_expr(n, part)
                }
                e => panic!("unexpected rule {:?} in deref lhs", e),
            };
            Expression::UnaryPre{
                op,
                loc,
                expr: Box::new(expr),
            }
        },
        Rule::call => {
            parse_call(n, expr)
        }
        Rule::macrocall => {
            let mut expr = expr.into_inner();
            let mut name = expr.next().unwrap().into_inner();
            let name = Name::from(name.next().unwrap().as_str());

            let mut args = Vec::new();
            if let Some(callargs) = expr.next() {
                for part in callargs.into_inner() {
                    args.push(Box::new(parse_expr(n, part)));
                }
            }

            Expression::MacroCall {
                loc,
                name,
                args,
            }

        },
        Rule::array_init => {
            let mut fields = Vec::new();
            let expr = expr.into_inner();
            for part in expr {
                match part.as_rule()  {
                    Rule::expr => {
                        let expr = parse_expr(n, part);
                        fields.push(Box::new(expr));
                    }
                    e => panic!("unexpected rule {:?} in struct init", e),
                }

            }
            Expression::ArrayInit{
                loc,
                fields,
            }
        }
        Rule::struct_init => {
            let mut expr = expr.into_inner();
            let part  = expr.next().unwrap();

            let typed = parse_anon_type(n, part);

            let mut fields = Vec::new();
            for part in expr {
                match part.as_rule()  {
                    Rule::struct_init_field => {
                        let mut part = part.into_inner();
                        let name = part.next().unwrap().as_str().to_string();
                        let expr = parse_expr(n, part.next().unwrap());
                        fields.push((name, Box::new(expr)));
                    }
                    e => panic!("unexpected rule {:?} in struct init", e),
                }

            }

            Expression::StructInit{
                loc,
                typed,
                fields,
            }
        }
        e => panic!("unexpected rule {:?} in expr", e),
    }
}

pub(crate) fn parse_statement(
    n: &str,
    features:   &HashMap<String, bool>,
    stage:      &Stage,
    stm:        pest::iterators::Pair<'static, Rule>,
    into:       &mut Vec<Box<Statement>>,
    current_if_statement: &mut Option<usize>,
) {

    let loc = Location::from_span(n.into(), &stm.as_span());
    match stm.as_rule() {
        Rule::mark_stm => {
            let mut stm = stm.into_inner();
            let part    = stm.next().unwrap();
            let lhs     = parse_expr(n, part);
            let part    = stm.next().unwrap();
            let mut part = part.into_inner();
            let key   = part.next().unwrap().as_str().into();
            let value = part.next().map(|s|s.as_str().into()).unwrap_or(String::new());

            into.push(Box::new(Statement::Mark{
                loc,
                lhs,
                key,
                value,
            }));
        },
        Rule::label => {
            let mut stm = stm.into_inner();
            let label   = stm.next().unwrap().as_str().to_string();
            into.push(Box::new(Statement::Label{
                loc,
                label,
            }));
        },
        Rule::continue_stm => {
            into.push(Box::new(Statement::Continue{
                loc,
            }));
        },
        Rule::break_stm => {
            into.push(Box::new(Statement::Break{
                loc,
            }));
        },
        Rule::block => {
            into.push(Box::new(Statement::Block(Box::new(parse_block(n, features, stage, stm)))))
        },
        Rule::return_stm  => {
            let mut stm = stm.into_inner();
            let key  = stm.next().unwrap();
            match key.as_rule() {
                Rule::key_return => { }
                a => { panic!("expected key_return instead of {:?}", a );}
            };
            let expr = if let Some(expr) = stm.next() {
                Some(parse_expr(n, expr))
            } else {
                None
            };
            into.push(Box::new(Statement::Return{
                expr,
                loc: loc.clone(),
            }));
        },
        Rule::expr => {
            let expr = parse_expr(n, stm);
            into.push(Box::new(Statement::Expr{
                expr,
                loc: loc.clone(),
            }));
        }
        Rule::while_stm => {
            let mut stm = stm.into_inner();
            let part    = stm.next().unwrap();
            let expr    = parse_expr(n, part);
            let part    = stm.next().unwrap();
            let body    = parse_block(n, features, stage, part);
            into.push(Box::new(Statement::While {
                expr,
                body,
            }));
        }
        Rule::if_stm => {
            let mut stm = stm.into_inner();
            let part    = stm.next().unwrap();
            let expr    = parse_expr(n, part);
            let part    = stm.next().unwrap();
            let body    = parse_block(n, features, stage, part);
            *current_if_statement = Some(into.len());
            into.push(Box::new(Statement::If{
                branches: vec![(loc.clone(), Some(expr), body)],
            }));
        }
        Rule::elseif_stm => {
            let mut stm = stm.into_inner();
            let part    = stm.next().unwrap();
            let expr    = parse_expr(n, part);
            let part    = stm.next().unwrap();
            let body    = parse_block(n, features, stage, part);
            match *current_if_statement {
                None => {
                    emit_error("else without if", &[
                        (loc.clone(), "this else branch does not follow an if condition")
                    ]);
                    std::process::exit(9);
                }
                Some(c) => {
                    if let Statement::If{ref mut branches} = *into[c] {
                        branches.push((loc.clone(), Some(expr), body));
                    }
                }
            }
        }
        Rule::else_stm => {
            let mut stm = stm.into_inner();
            let part    = stm.next().unwrap();
            let body    = parse_block(n, features, stage, part);
            match *current_if_statement {
                None => {
                    emit_error("else without if", &[
                        (loc.clone(), "this else branch does not follow an if condition")
                    ]);
                    std::process::exit(9);
                }
                Some(c) => {
                    if let Statement::If{ref mut branches} = *into[c] {
                        branches.push((loc.clone(), None, body));
                    }
                }
            }
            *current_if_statement = None;
        }
        Rule::for_stm => {



            let stm = stm.into_inner();

            let mut expr1 = Vec::new();
            let mut expr2 = None;
            let mut expr3 = Vec::new();
            let mut block = None;

            let mut cur = 1;

            for part in stm {
                match part.as_rule() {
                    Rule::semicolon => {
                        cur += 1;
                    },
                    Rule::block if cur == 3 && block.is_none() => {
                        block = Some(parse_block(n, features, stage, part));
                    },
                    _ if cur == 1 => {
                        let mut cif = None;
                        parse_statement(n, features, stage, part, &mut expr1, &mut cif);
                    },
                    _ if cur == 2 => {
                        expr2 = Some(parse_expr(n, part));
                    },
                    _ if cur == 3 => {
                        let mut cif = None;
                        parse_statement(n, features, stage, part, &mut expr3, &mut cif);
                    },
                    e => panic!("unexpected rule {:?} in for ", e),
                }
            }

            into.push(Box::new(Statement::For{
                e1:     expr1,
                e2:     expr2,
                e3:     expr3,
                body:   block.unwrap(),
            }));
        }
        Rule::vardecl => {
            let stm = stm.into_inner();
            let mut typed   = None;
            let mut assign  = None;
            let mut array   = None;

            for part in stm {
                match part.as_rule() {
                    Rule::named_type => {
                        typed = Some(parse_named_type(n, part));
                    },
                    Rule::expr => {
                        assign = Some(parse_expr(n, part));
                    }
                    Rule::array => {
                        if let Some(expr) = part.into_inner().next() {
                            array = Some(Some(parse_expr(n, expr)));
                        } else {
                            array = Some(None);
                        }
                    }
                    e => panic!("unexpected rule {:?} in vardecl", e),
                }
            }

            let TypedName{typed, name, tags} = typed.unwrap();

            into.push(Box::new(Statement::Var{
                loc: loc.clone(),
                typed,
                name,
                tags,
                array,
                assign,
            }))
        }
        Rule::assign => {
            let stm = stm.into_inner();
            let mut lhs     = None;
            let mut rhs     = None;
            let mut op      = None;

            for part in stm {
                match part.as_rule() {
                    Rule::expr if lhs.is_none() => {
                        lhs = Some(parse_expr(n, part));
                    }
                    Rule::assignop => {
                        op = Some(match part.into_inner().next().unwrap().as_rule() {
                            Rule::assignbitor  => AssignOperator::Bitor,
                            Rule::assignbitand => AssignOperator::Bitand,
                            Rule::assignadd    => AssignOperator::Add,
                            Rule::assignsub    => AssignOperator::Sub,
                            Rule::assigneq     => AssignOperator::Eq,
                            _ => {
                                emit_error("ICE: unexpected operator", &[
                                    (loc.clone(), "in this assign expr")
                                ]);
                                std::process::exit(9);
                            }
                        });
                    }
                    Rule::expr if rhs.is_none() => {
                        rhs = Some(parse_expr(n, part));
                    }
                    e => panic!("unexpected rule {:?} in assign", e),
                }
            }

            into.push(Box::new(Statement::Assign{
                loc:    loc.clone(),
                lhs:    lhs.unwrap(),
                rhs:    rhs.unwrap(),
                op:     op.unwrap(),
            }))
        }
        Rule::switch_stm => {
            let mut stm  = stm.into_inner();
            let mut default = None;
            let expr = parse_expr(n, stm.next().unwrap());

            let mut cases = Vec::new();

            for part in stm {
                let mut part = part.into_inner();
                let ppart = part.next().unwrap();
                if ppart.as_rule() == Rule::key_default {
                    if default.is_some() {
                        emit_error("multiple default cases", &[
                            (loc.clone(), "in this switch")
                        ]);
                        std::process::exit(9);
                    } else {
                        default = Some(parse_block(n, features,  stage,part.next().unwrap()));
                    }
                } else {
                    let mut case_cond = Vec::new();
                    for case in ppart.into_inner() {
                        case_cond.push(parse_expr(n, case));
                    }

                    let block = parse_block(n, features,  stage,part.next().unwrap());
                    cases.push((case_cond,block));
                }
            }

            into.push(Box::new(Statement::Switch {
                default,
                loc,
                expr,
                cases,
            }))
        },
        Rule::unsafe_block => {
            into.push(Box::new(Statement::Unsafe(Box::new(parse_block(n, features, stage, stm.into_inner().next().unwrap())))));
        },
        Rule::cblock => {
            let stm = stm.into_inner().next().unwrap();
            let loc = Location::from_span(n.into(), &stm.as_span());
            into.push(Box::new(Statement::CBlock{
                loc,
                lit: stm.as_str().to_string()
            }));
        },
        e => panic!("unexpected rule {:?} in block", e),
    }
}

pub(crate) fn parse_block(
        n:          &str,
        features:   &HashMap<String,bool>,
        stage:      &Stage,
        decl:       pest::iterators::Pair<'static, Rule>
) -> Block {
    match decl.as_rule() {
        Rule::block => { }
        _ => { panic!("parse_block called with {:?}", decl); }
    };

    let end = Location{
        file:   n.into(),
        start:  decl.as_span().end(),
        end:    decl.as_span().end(),
        line:   decl.as_span().end_pos().line_col().0,
    };

    let mut statements = Vec::new();
    let mut cif_state = None;
    for stm in PP::new(n, features.clone(), stage.clone(), decl.into_inner()) {
        parse_statement(n, features, stage, stm, &mut statements, &mut cif_state)
    }
    Block{
        statements,
        end,
        expanded: false,
    }
}


// typed is parsed left to right

#[derive(Debug)]
pub(crate) struct TypedName {
    name:   String,
    typed:  Typed,
    tags:   Tags,
}

pub(crate) fn parse_named_type(n: &str, decl: pest::iterators::Pair<'static, Rule>) -> TypedName {
    match decl.as_rule() {
        Rule::named_type => { }
        _ => { panic!("parse_named_type called with {:?}", decl); }
    };

    let loc = Location::from_span(n.into(), &decl.as_span());

    let mut tail = Tail::None;

    //the actual type name is always on the left hand side
    let mut decl = decl.into_inner();
    let mut lhsdecl = decl.next().unwrap().into_inner();
    let typename = Name::from(lhsdecl.next().unwrap().as_str());
    for lhs in lhsdecl {
        match lhs.as_rule() {
            Rule::tail => {
                let loc = Location::from_span(n.into(), &lhs.as_span());
                let mut part = lhs.as_str().to_string();
                part.remove(0);
                if part.len() > 0 {
                    if let Ok(n) = part.parse::<u64>() {
                        tail = Tail::Static(n, loc);
                    } else {
                        tail = Tail::Bind(part, loc);
                    }
                } else {
                    tail = Tail::Dynamic
                }
            },
            e => panic!("unexpected rule {:?} in named_type lhs", e),
        }
    }

    // the local variable name is on the right;
    let mut decl : Vec<pest::iterators::Pair<'static, Rule>> = decl.collect();
    let name_part = decl.pop().unwrap();
    let name = match name_part.as_rule() {
        Rule::ident => {
            let name = name_part.as_str().to_string();
            if name == "return" {
                let loc = Location::from_span(n.into(), &name_part.as_span());
                emit_error("syntax error", &[
                    (loc, "llegal use of keyword 'return'"),
                ]);
                std::process::exit(9);
            }
            name
        }
        _ => {
            let loc = Location::from_span(n.into(), &name_part.as_span());
            emit_error("syntax error", &[
                (loc.clone(), "expected a name")
            ]);
            std::process::exit(9);
        }
    };

    let mut tags = Tags::new();
    let mut ptr = Vec::new();

    for part in decl {
        let loc = Location::from_span(n.into(), &part.as_span());
        match part.as_rule() {
            Rule::ptr => {
                ptr.push(Pointer{
                    tags: std::mem::replace(&mut tags, Tags::new()),
                    loc,
                });
            }
            Rule::tag_name => {
                let mut part = part.into_inner();
                let mut name  = part.next().unwrap().as_str().into();
                if name == "mutable" {
                    name = "mut".to_string();
                }
                let value = part.next().as_ref().map(|s|s.as_str().to_string()).unwrap_or(String::new());
                tags.insert(name, value, loc);
            }
            e => panic!("unexpected rule {:?} in named_type ", e),
        }
    }



    TypedName {
        name,
        typed: Typed {
            t:   Type::Other(typename),
            loc: loc.clone(),
            ptr,
            tail,
        },
        tags,
    }
}

pub(crate) fn parse_anon_type(n: &str, decl: pest::iterators::Pair<'static, Rule>) -> Typed {
    match decl.as_rule() {
        Rule::anon_type => { }
        _ => { panic!("parse_anon_type called with {:?}", decl); }
    };
    let loc = Location::from_span(n.into(), &decl.as_span());
    //the actual type name is always on the left hand side
    let mut decl = decl.into_inner();
    let name = Name::from(decl.next().unwrap().as_str());

    let mut tags = Tags::new();
    let mut ptr  = Vec::new();
    let mut tail = Tail::None;

    for part in decl {
        let loc = Location::from_span(n.into(), &part.as_span());
        match part.as_rule() {
            Rule::ptr => {
                ptr.push(Pointer{
                    tags: std::mem::replace(&mut tags, Tags::new()),
                    loc,
                });
            }
            Rule::tag_name => {
                let mut part = part.into_inner();
                let mut name  = part.next().unwrap().as_str().into();
                if name == "mutable" {
                    name = "mut".to_string();
                }
                let value = part.next().as_ref().map(|s|s.as_str().to_string()).unwrap_or(String::new());
                tags.insert(name, value, loc);
            }
            Rule::tail => {
                let loc = Location::from_span(n.into(), &part.as_span());
                let mut part = part.as_str().to_string();
                part.remove(0);
                if part.len() > 0 {
                    if let Ok(n) = part.parse::<u64>() {
                        tail = Tail::Static(n, loc);
                    } else {
                        tail = Tail::Bind(part, loc);
                    }
                } else {
                    tail = Tail::Dynamic
                }
            },
            e => panic!("unexpected rule {:?} in anon_type", e),
        }
    }

    for (_,tag) in tags.0 {
        emit_error("syntax error", &[
            (tag.iter().next().unwrap().1.clone(), "anonymous type cannot have storage tags (yet)"),
        ]);
        std::process::exit(9);
    }

    Typed {
        t: Type::Other(name),
        loc, ptr, tail,
    }
}


pub(crate) fn parse_importname(decl: pest::iterators::Pair<Rule>) -> (Name, Vec<(String, Option<String>)>) {
    let mut locals = Vec::new();
    let mut v = Vec::new();
    for part in decl.into_inner() {
        match part.as_rule() {
            Rule::cimport => {
                v = vec![String::new(), "ext".into(), part.as_str().into()];
            }
            Rule::ident => {
                v.push(part.as_str().into());
            }
            Rule::local => {
                for p2 in part.into_inner() {
                    match p2.as_rule() {
                        Rule::local_i => {
                            let mut p2      = p2.into_inner();
                            let name        = p2.next().unwrap();
                            let name = match name.as_rule() {
                                Rule::ident => {
                                    name.as_str().to_string()
                                }
                                Rule::qident => {
                                    name.into_inner().next().unwrap().as_str().to_string()
                                },
                                _ => unreachable!(),
                            };
                            let import_as   = if let Some(p3) = p2.next() {
                                Some(p3.as_str().to_string())
                            } else {
                                None
                            };
                            locals.push((name, import_as));
                        },
                        e => panic!("unexpected rule {:?} in local", e)
                    }
                }
            },
            Rule::type_name | Rule::importname => {
                let (name, locals2) = parse_importname(part);
                v.extend(name.0);
                locals.extend(locals2);
            }
            e => panic!("unexpected rule {:?} in import name ", e),
        }
    }
    (Name(v), locals)
}

fn parse_call(n: &str, expr: pest::iterators::Pair<'static, Rule>) -> Expression {
    let loc = Location::from_span(n.into(), &expr.as_span());
    let expr = expr.into_inner();
    //let name = expr.next().unwrap();
    //let nameloc = Location{
    //    file: n.into(),
    //    span: name.as_span(),
    //};
    //let name = Box::new(parse_expr(n, name));


    let mut args = Vec::new();


    for part in expr.into_iter() {
        match part.as_rule() {
            Rule::call_args => {
                args = part.into_inner().into_iter().map(|arg|{
                    Box::new(parse_expr(n, arg))
                }).collect();
            },
            e => panic!("unexpected rule {:?} in function call", e),
        }
    };

    Expression::Call{
        name : Box::new(Expression::Literal{
            v: "#error ICE this was supposed to be removed by pre climber pass".to_string(),
            loc: loc.clone()
        }),
        loc: loc,
        args,
        expanded:       false,
        emit:           EmitBehaviour::Default,
    }
}

use serde::{Serialize};

#[derive(Serialize, Default)]
pub struct JsonError {
    pub message:        String,
    pub level:          String,
    pub file_name:      String,
    pub line_start:     usize,
    pub line_end:       usize,
    pub column_start:   usize,
    pub column_end:     usize,
}

pub fn emit_error<'a, S1, S2, I>(message: S1, v: I)
    where S1: std::string::ToString,
          S2: std::string::ToString + 'a,
          I:  std::iter::IntoIterator<Item=&'a (Location, S2)>,
{
    if ERRORS_AS_JSON.load(Ordering::SeqCst) {
        let mut j = JsonError::default();
        j.message   = message.to_string();
        j.level     = "error".to_string();
        j.file_name = "<anon>".to_string();


        let mut first  = true;
        for (loc, message) in v.into_iter() {
            let span = loc.to_span();
            j.file_name     = loc.file.clone();
            j.line_start    = span.start_pos().line_col().0;
            j.column_start  = span.start_pos().line_col().1;
            j.line_end      = span.end_pos().line_col().0;
            j.column_end    = span.end_pos().line_col().1;


            if first {
                println!("{}", serde_json::to_string(&j).unwrap());
                first = false;
            }

            j.level     = "W".to_string();
            j.message   = message.to_string();
            println!("{}", serde_json::to_string(&j).unwrap());
        }


        return;
    }


    let mut s : String = message.to_string();
    for (loc, message)  in v.into_iter() {
        let span = loc.to_span();

        let e = pest::error::Error::<Rule>::new_from_span(pest::error::ErrorVariant::CustomError {
            message: message.to_string(),
        }, span).with_path(&loc.file);
        s += &format!("\n{}\n", e);
    }
    error!("{}", s);
}

pub fn emit_warn<'a, S1, S2, I>(message: S1, v: I)
    where S1: std::string::ToString,
          S2: std::string::ToString + 'a,
          I:  std::iter::IntoIterator<Item=&'a (Location, S2)>,
{
    if ERRORS_AS_JSON.load(Ordering::SeqCst) {
        let mut j = JsonError::default();
        j.message   = message.to_string();
        j.level     = "warn".to_string();
        j.file_name = "<anon>".to_string();

        if let Some((loc,_)) = v.into_iter().next() {
            let span = loc.to_span();
            j.file_name     = loc.file.clone();
            j.line_start    = span.start_pos().line_col().0;
            j.column_start  = span.start_pos().line_col().1;
            j.line_end      = span.end_pos().line_col().0;
            j.column_end    = span.end_pos().line_col().1;
        }

        println!("{}", serde_json::to_string(&j).unwrap());
        return;
    }

    let mut s : String = message.to_string();
    for (loc, message)  in v.into_iter() {
        let span = loc.to_span();
        let e = pest::error::Error::<Rule>::new_from_span(pest::error::ErrorVariant::CustomError {
            message: message.to_string(),
        }, span).with_path(&loc.file);
        s += &format!("\n{}", e);
    }
    warn!("{}", s);
}

pub fn emit_debug<'a, S1, S2, I>(message: S1, v: I)
    where S1: std::string::ToString,
          S2: std::string::ToString + 'a,
          I:  std::iter::IntoIterator<Item=&'a (Location, S2)>,
{
    if ERRORS_AS_JSON.load(Ordering::SeqCst) {
        return;
    }


    let mut s : String = message.to_string();
    for (loc, message)  in v.into_iter() {
        let span = loc.to_span();
        let e = pest::error::Error::<Rule>::new_from_span(pest::error::ErrorVariant::CustomError {
            message: message.to_string(),
        }, span).with_path(&loc.file);
        s += &format!("\n{}", e);
    }
    debug!("{}", s);
}

fn unescape(s: &str, loc: &Location) -> Vec<u8> {
    let mut result = Vec::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        result.push(
            if ch != '\\' {
                ch as u8
            } else {
                match chars.next() {
                    Some('x') => {
                        let value = chars.by_ref().take(2).fold(0, |acc, c| acc * 16 + c.to_digit(16).unwrap());
                        if value > 255 {
                            emit_error("octal value too big for char", &[
                                (loc.clone(), "in this literal string")
                            ]);
                            std::process::exit(9);
                        }
                        value as u8
                    }
                    Some('?') => 0x3f,
                    Some('\\')=> '\\' as u8,
                    Some('a') => 0x07,
                    Some('b') => 0x08,
                    Some('f') => 0x0c,
                    Some('n') => '\n' as u8,
                    Some('r') => '\r' as u8,
                    Some('t') => '\t' as u8,
                    Some('v') => 0x0b,
                    Some('"') => '"' as u8,
                    Some('\'') => '\'' as u8,
                    _ => {
                        emit_error("unsupported escape character", &[
                            (loc.clone(), "in this literal string")
                        ]);
                        std::process::exit(9);
                    }
                }
            }
        )
    }
    result
}



pub fn parse_u64(s: &str) -> Option<u64> {
    if s.len() > 2 && s.chars().nth(0) == Some('0') && s.chars().nth(1) == Some('x') {
        return u64::from_str_radix(&s[2..], 16).ok();
    }

    if let Ok(v) = s.parse::<u64>() {
        return Some(v)
    }

    None
}
