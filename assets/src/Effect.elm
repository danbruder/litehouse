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
    , GitHubStatus(..)
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
    | Load String
      -- Ports
    | SaveToken String
    | SaveRefreshToken String
    | ClearToken
    | GetRefreshToken
    | ConnectSSE
        { token : String
        , filters : Maybe Encode.Value
        }
    | DisconnectSSE
    | UpdateSSEFilters (Maybe Encode.Value)
      -- HTTP Requests - Auth
    | CheckServerStatus (Result String ServerStatus -> msg)
    | VerifyToken String (Result String { user : UserInfo, token : String } -> msg)
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
    | StartLogStreaming String String (Result String String -> msg)
    | FetchBuilds String String (Result String (List BuildInfo) -> msg)
    | FetchBuildLogs String String String (Result String String -> msg)
      -- HTTP Requests - GitHub
    | FetchGitHubStatus String (Result String GitHubStatusResponse -> msg)
    | StartDeviceFlow String (Result String DeviceFlowStartResponse -> msg)
    | StartGitHubPolling String String Int Int  -- token, deviceCode, interval, expiresIn
    | FetchRepos String (Result String (List RepoInfo) -> msg)
    | CreateApp String String (Result String AppInfo -> msg)
    | CreateAppWithRepo String String String (Result String AppInfo -> msg)
      -- Shared state updates
    | UpdateGitHubStatus GitHubStatus


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
    , imageTag : Maybe String
    , gitCommit : Maybe String
    , status : String
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


type GitHubStatus
    = GitHubUnknown
    | GitHubNotConnected
    | GitHubConnected String


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

        Load href ->
            Load href

        SaveToken token ->
            SaveToken token

        SaveRefreshToken token ->
            SaveRefreshToken token

        ClearToken ->
            ClearToken

        GetRefreshToken ->
            GetRefreshToken

        ConnectSSE config ->
            ConnectSSE config

        DisconnectSSE ->
            DisconnectSSE

        UpdateSSEFilters filters ->
            UpdateSSEFilters filters

        StartGitHubPolling token deviceCode interval expiresIn ->
            StartGitHubPolling token deviceCode interval expiresIn

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

        StartLogStreaming token appName toMsg ->
            StartLogStreaming token appName (toMsg >> fn)

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

        UpdateGitHubStatus status ->
            UpdateGitHubStatus status
