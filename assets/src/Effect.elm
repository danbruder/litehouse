module Effect exposing
    ( Effect(..)
    , none
    , batch
    , map
    -- Expose types used by pages
    , ServerStatus
    , TokenVerificationResponse
    , UserInfo
    , TokenPair
    , AuthResponse
    , LoginForm
    , SetupForm
    , AppInfo
    , AppDetail
    , RemoteInfo
    , BuildInfo
    , GitHubStatusResponse
    , DeviceFlowStartResponse
    , RepoInfo
    )

{-| The Effect pattern allows us to return a custom type from update functions
instead of Cmd, making our side effects transparent, testable, and inspectable.

This Effect type represents all possible side effects in the application.
-}

import Json.Decode as Decode
import Json.Encode as Encode


-- EFFECT TYPE


type Effect msg
    = None
    | Batch (List (Effect msg))
      -- Navigation
    | PushUrl String
    | ReplaceUrl String
      -- Ports
    | SaveToken String
    | SaveRefreshToken String
    | ClearToken
    | GetRefreshToken
    | StartGitHubSSE
        { token : String
        , deviceCode : String
        , interval : Int
        , expiresIn : Int
        }
      -- HTTP Requests - Auth
    | CheckServerStatus (Result String ServerStatus -> msg)
    | VerifyToken String (Result String TokenVerificationResponse -> msg)
    | RefreshAccessToken String (Result String TokenPair -> msg)
    | SubmitLogin LoginForm (Result String AuthResponse -> msg)
    | SubmitRegister SetupForm (Result String AuthResponse -> msg)
      -- HTTP Requests - Apps
    | FetchApps String (Result String (List AppInfo) -> msg)
    | FetchAppDetail String String (Result String AppDetail -> msg)
    | StartApp String String (Result String String -> msg)
    | StopApp String String (Result String String -> msg)
    | BuildApp String String (Result String String -> msg)
    | DeleteApp String String (Result String String -> msg)
    | FetchLogs String String (Result String String -> msg)
    | FetchBuilds String String (Result String (List BuildInfo) -> msg)
    | FetchBuildLogs String String String (Result String String -> msg)
      -- HTTP Requests - GitHub
    | FetchGitHubStatus String (Result String GitHubStatusResponse -> msg)
    | StartDeviceFlow String (Result String DeviceFlowStartResponse -> msg)
    | FetchRepos String (Result String (List RepoInfo) -> msg)
    | CreateApp String String (Result String AppInfo -> msg)
    | CreateAppWithRepo String String String (Result String AppInfo -> msg)


-- HELPER TYPES (mirrored from Main.elm for now)


type alias ServerStatus =
    { initialized : Bool
    , version : String
    }


type alias TokenVerificationResponse =
    { user : UserInfo
    , token : String
    }


type alias UserInfo =
    { email : String
    , fullName : String
    }


type alias TokenPair =
    { accessToken : String
    , refreshToken : String
    }


type alias AuthResponse =
    { accessToken : String
    , refreshToken : String
    , user : UserInfo
    }


type alias LoginForm =
    { email : String
    , password : String
    }


type alias SetupForm =
    { email : String
    , password : String
    , confirmPassword : String
    , fullName : String
    , organizationName : String
    }


type alias AppInfo =
    { id : String
    , name : String
    , state : String
    }


type alias AppDetail =
    { id : String
    , name : String
    , state : String
    , port_ : Maybe Int
    , createdAt : String
    , updatedAt : String
    , remote : Maybe RemoteInfo
    }


type alias RemoteInfo =
    { name : String
    , url : String
    , branch : String
    }


type alias BuildInfo =
    { id : String
    , appId : String
    , imageTag : String
    , gitCommit : String
    , createdAt : String
    }


type alias GitHubStatusResponse =
    { connected : Bool
    , username : Maybe String
    }


type alias DeviceFlowStartResponse =
    { userCode : String
    , verificationUri : String
    , deviceCode : String
    , expiresIn : Int
    , interval : Int
    }


type alias RepoInfo =
    { name : String
    , fullName : String
    , description : Maybe String
    , private : Bool
    , cloneUrl : String
    , defaultBranch : String
    }


-- CONSTRUCTORS


none : Effect msg
none =
    None


batch : List (Effect msg) -> Effect msg
batch =
    Batch


-- MAPPING


map : (a -> b) -> Effect a -> Effect b
map fn effect =
    case effect of
        None ->
            None

        Batch effects ->
            Batch (List.map (map fn) effects)

        PushUrl url ->
            PushUrl url

        ReplaceUrl url ->
            ReplaceUrl url

        SaveToken token ->
            SaveToken token

        SaveRefreshToken token ->
            SaveRefreshToken token

        ClearToken ->
            ClearToken

        GetRefreshToken ->
            GetRefreshToken

        StartGitHubSSE config ->
            StartGitHubSSE config

        CheckServerStatus toMsg ->
            CheckServerStatus (toMsg >> fn)

        VerifyToken token toMsg ->
            VerifyToken token (toMsg >> fn)

        RefreshAccessToken token toMsg ->
            RefreshAccessToken token (toMsg >> fn)

        SubmitLogin form toMsg ->
            SubmitLogin form (toMsg >> fn)

        SubmitRegister form toMsg ->
            SubmitRegister form (toMsg >> fn)

        FetchApps token toMsg ->
            FetchApps token (toMsg >> fn)

        FetchAppDetail token appName toMsg ->
            FetchAppDetail token appName (toMsg >> fn)

        StartApp token appName toMsg ->
            StartApp token appName (toMsg >> fn)

        StopApp token appName toMsg ->
            StopApp token appName (toMsg >> fn)

        BuildApp token appName toMsg ->
            BuildApp token appName (toMsg >> fn)

        DeleteApp token appName toMsg ->
            DeleteApp token appName (toMsg >> fn)

        FetchLogs token appName toMsg ->
            FetchLogs token appName (toMsg >> fn)

        FetchBuilds token appName toMsg ->
            FetchBuilds token appName (toMsg >> fn)

        FetchBuildLogs token appName buildId toMsg ->
            FetchBuildLogs token appName buildId (toMsg >> fn)

        FetchGitHubStatus token toMsg ->
            FetchGitHubStatus token (toMsg >> fn)

        StartDeviceFlow token toMsg ->
            StartDeviceFlow token (toMsg >> fn)

        FetchRepos token toMsg ->
            FetchRepos token (toMsg >> fn)

        CreateApp token name toMsg ->
            CreateApp token name (toMsg >> fn)

        CreateAppWithRepo token name repo toMsg ->
            CreateAppWithRepo token name repo (toMsg >> fn)
