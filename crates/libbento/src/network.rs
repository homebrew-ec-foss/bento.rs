#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkMode {
     Slirp,
     Bridge,
     Host, 
     None
}


impl std::str::FromStr for NetworkMode {
      type Err = String;

      fn from_str(s : &str) -> Result<Self, Self::Err> {
                    match s.to_lowercase().as_str() {
                                "slirp" => Ok(Self::Slirp),
                                "bridge" => Ok(Self::Bridge),
                                "host" => Ok(Self::Host),
                                "none" => Ok(Self::None),
                                _ => Err(format!("Unknown net mode : {}. ", s))
                    }
      }
}

// this func can be used to return or logging : NetworkMode::Slirp == "slirp"
impl std::fmt::Display for NetworkMode {
      fn fmt (&self, f : &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    let s = match self {
                            Self::Slirp => "slirp",
                            Self::Bridge => "bridge",
                            Self::Host=> "host",
                            Self::None=> "none",
                    };
                    write!(f, "{}", s)
      }
}



