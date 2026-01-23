module Page.Setup exposing
    ( Model
    , Msg
    , init
    , update
    , view
    , save
    , load
    )

import Effect exposing (Effect)
import Html exposing (Html, button, div, h1, input, label, p, text)
import Html.Attributes exposing (class, disabled, for, id, placeholder, required, type_, value)
import Html.Events exposing (onInput, onSubmit)
import Shared


-- MODEL


type alias Model =
    { email : String
    , password : String
    , confirmPassword : String
    , fullName : String
    , organizationName : String
    , error : Maybe String
    , submitting : Bool
    }


-- INIT


init : Shared.Model -> ( Model, Effect Msg )
init shared =
    ( { email = ""
      , password = ""
      , confirmPassword = ""
      , fullName = ""
      , organizationName = ""
      , error = Nothing
      , submitting = False
      }
    , Effect.none
    )


-- UPDATE


type Msg
    = EmailChanged String
    | PasswordChanged String
    | ConfirmPasswordChanged String
    | FullNameChanged String
    | OrganizationNameChanged String
    | Submit
    | GotRegisterResponse (Result String Effect.AuthResponse)


update : Shared.Model -> Msg -> Model -> ( Model, Effect Msg )
update shared msg model =
    case msg of
        EmailChanged email ->
            ( { model | email = email }
            , Effect.none
            )

        PasswordChanged password ->
            ( { model | password = password }
            , Effect.none
            )

        ConfirmPasswordChanged confirmPassword ->
            ( { model | confirmPassword = confirmPassword }
            , Effect.none
            )

        FullNameChanged fullName ->
            ( { model | fullName = fullName }
            , Effect.none
            )

        OrganizationNameChanged orgName ->
            ( { model | organizationName = orgName }
            , Effect.none
            )

        Submit ->
            if model.password /= model.confirmPassword then
                ( { model | error = Just "Passwords do not match" }
                , Effect.none
                )

            else if String.length model.password < 8 then
                ( { model | error = Just "Password must be at least 8 characters" }
                , Effect.none
                )

            else
                ( { model | submitting = True, error = Nothing }
                , Effect.SubmitRegister
                    { email = model.email
                    , password = model.password
                    , confirmPassword = model.confirmPassword
                    , fullName = model.fullName
                    , organizationName = model.organizationName
                    }
                    GotRegisterResponse
                )

        GotRegisterResponse result ->
            case result of
                Ok response ->
                    -- Registration successful - navigation and token saving will be handled by Main
                    ( { model | submitting = False }
                    , Effect.batch
                        [ Effect.SaveToken response.accessToken
                        , Effect.SaveRefreshToken response.refreshToken
                        , Effect.PushUrl "/dashboard"
                        ]
                    )

                Err error ->
                    ( { model
                        | submitting = False
                        , error = Just error
                      }
                    , Effect.none
                    )


-- VIEW


view : Shared.Model -> Model -> Html Msg
view shared model =
    div [ class "min-h-screen bg-litehouse-bg flex items-center justify-center p-5" ]
        [ div [ class "bg-litehouse-surface rounded-2xl shadow-soft p-10 w-full max-w-md text-center" ]
            [ h1 [ class "text-2xl font-semibold text-litehouse-text mb-2" ] [ text "Welcome to Litehouse" ]
            , p [ class "text-litehouse-muted mb-6" ] [ text "Create your admin account to get started" ]
            , Html.form [ onSubmit Submit ]
                [ viewError model.error
                , div [ class "mb-4 text-left" ]
                    [ label [ for "fullName", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "Full Name" ]
                    , input
                        [ type_ "text"
                        , id "fullName"
                        , value model.fullName
                        , onInput FullNameChanged
                        , placeholder "John Doe"
                        , required True
                        , disabled model.submitting
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:bg-litehouse-bg disabled:cursor-not-allowed"
                        ]
                        []
                    ]
                , div [ class "mb-4 text-left" ]
                    [ label [ for "email", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "Email" ]
                    , input
                        [ type_ "email"
                        , id "email"
                        , value model.email
                        , onInput EmailChanged
                        , placeholder "admin@example.com"
                        , required True
                        , disabled model.submitting
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:bg-litehouse-bg disabled:cursor-not-allowed"
                        ]
                        []
                    ]
                , div [ class "mb-4 text-left" ]
                    [ label [ for "orgName", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "Organization Name" ]
                    , input
                        [ type_ "text"
                        , id "orgName"
                        , value model.organizationName
                        , onInput OrganizationNameChanged
                        , placeholder "My Organization"
                        , required True
                        , disabled model.submitting
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:bg-litehouse-bg disabled:cursor-not-allowed"
                        ]
                        []
                    ]
                , div [ class "mb-4 text-left" ]
                    [ label [ for "password", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "Password" ]
                    , input
                        [ type_ "password"
                        , id "password"
                        , value model.password
                        , onInput PasswordChanged
                        , placeholder "At least 8 characters"
                        , required True
                        , disabled model.submitting
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:bg-litehouse-bg disabled:cursor-not-allowed"
                        ]
                        []
                    ]
                , div [ class "mb-4 text-left" ]
                    [ label [ for "confirmPassword", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "Confirm Password" ]
                    , input
                        [ type_ "password"
                        , id "confirmPassword"
                        , value model.confirmPassword
                        , onInput ConfirmPasswordChanged
                        , placeholder "Re-enter your password"
                        , required True
                        , disabled model.submitting
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:bg-litehouse-bg disabled:cursor-not-allowed"
                        ]
                        []
                    ]
                , button
                    [ type_ "submit"
                    , class "w-full px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors mt-2 disabled:bg-litehouse-border disabled:cursor-not-allowed"
                    , disabled model.submitting
                    ]
                    [ text
                        (if model.submitting then
                            "Creating Account..."

                         else
                            "Create Account"
                        )
                    ]
                ]
            , viewVersion shared.serverVersion
            ]
        ]


viewError : Maybe String -> Html msg
viewError error =
    case error of
        Just err ->
            div [ class "mb-4 p-3 bg-red-50 border border-red-200 rounded-xl text-sm text-red-700" ]
                [ text err ]

        Nothing ->
            text ""


viewVersion : String -> Html msg
viewVersion version =
    if String.isEmpty version then
        text ""

    else
        p [ class "text-xs text-litehouse-muted mt-4" ]
            [ text ("v" ++ version) ]


-- PAGE LIFECYCLE


{-| Save page state to global state when leaving the page
-}
save : Model -> Shared.Model -> Shared.Model
save model shared =
    -- Nothing to save for setup page
    shared


{-| Load data from global state when entering the page
-}
load : Shared.Model -> Model -> Model
load shared model =
    -- Nothing to load for setup page
    model
